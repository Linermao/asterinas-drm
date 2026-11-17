// SPDX-License-Identifier: MPL-2.0

use alloc::{collections::VecDeque, vec::Vec};
use core::{fmt::Debug, mem, time::Duration};

use aster_core::{prelude::*, process::signal::Pause};
use ostd::sync::{Mutex, WaitQueue};

use crate::event::{DrmEventBase, DrmEventReservation, DrmEventResponse, DrmEventType};

pub mod software;

// Preserves legacy 2^23-frame comparison window to distinguish
// reached targets from future targets across counter wraparound.
pub(crate) const MAX_PASSED_DISTANCE: u64 = 1 << 23;

/// A vblank request with reserved response capacity.
#[derive(Debug)]
pub(crate) struct DrmVblankRequest {
    event_type: DrmEventType,
    target_sequence: u64,
    user_data: u64,
    crtc_id: u32,
    reservation: DrmEventReservation,
}

impl DrmVblankRequest {
    pub(crate) fn new(
        target_sequence: u64,
        user_data: u64,
        crtc_id: u32,
        reservation: DrmEventReservation,
    ) -> Self {
        Self {
            event_type: DrmEventType::Vblank,
            target_sequence,
            user_data,
            crtc_id,
            reservation,
        }
    }
}

/// `struct drm_event_vblank` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/drm/drm.h#L1369-L1376>.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(crate) struct DrmVblankResponse {
    base: DrmEventBase,
    user_data: u64,
    tv_sec: u32,
    tv_usec: u32,
    sequence: u32,
    crtc_id: u32,
}

impl DrmVblankResponse {
    fn new(
        event_type: DrmEventType,
        user_data: u64,
        crtc_id: u32,
        sequence: u64,
        timestamp: Duration,
    ) -> Self {
        let base = DrmEventBase::new(event_type, size_of::<Self>() as u32);

        Self {
            base,
            user_data,
            tv_sec: timestamp.as_secs() as u32,
            tv_usec: timestamp.subsec_micros(),
            // Preserves the legacy ABI's 32-bit modular sequence counter.
            sequence: sequence as u32,
            crtc_id,
        }
    }
}

#[derive(Debug)]
struct DrmVblankCompletion {
    response: DrmVblankResponse,
    reservation: DrmEventReservation,
}

impl DrmVblankCompletion {
    fn new(request: DrmVblankRequest, sequence: u64, timestamp: Duration) -> Self {
        Self {
            response: DrmVblankResponse::new(
                request.event_type,
                request.user_data,
                request.crtc_id,
                sequence,
                timestamp,
            ),
            reservation: request.reservation,
        }
    }

    fn deliver(self) {
        self.reservation
            .consume_with_response(DrmEventResponse::Vblank(self.response))
    }
}

/// Per-CRTC vblank state, event delivery, and synchronous waits.
///
/// KMS commits explicitly start and stop vblank generation.
///
/// Lock order: modeset state, vblank state. Event delivery and synchronous
/// waiter notification happen after releasing the vblank state lock.
pub struct DrmVblank {
    state: Mutex<DrmVblankState>,
    wait_queue: WaitQueue,
}

impl Default for DrmVblank {
    fn default() -> Self {
        Self {
            state: Mutex::new(DrmVblankState::default()),
            wait_queue: WaitQueue::new(),
        }
    }
}

impl DrmVblank {
    pub fn enable(&self) {
        self.state.lock().is_enabled = true;
    }

    /// Disables vblank reporting and completes pending requests with the last snapshot.
    pub fn disable(&self) {
        let (sequence, timestamp, requests) = {
            let mut state = self.state.lock();
            state.is_enabled = false;
            state.pending_flip = None;
            let requests = mem::take(&mut state.requests);
            (state.sequence, state.timestamp, requests)
        };

        self.wait_queue.wake_all();
        for request in requests {
            DrmVblankCompletion::new(request, sequence, timestamp).deliver();
        }
    }

    pub(crate) fn sequence(&self) -> u64 {
        self.state.lock().sequence
    }

    /// Checks whether a legacy flip can be accepted while the modeset lock is held.
    pub(crate) fn check_page_flip(&self) -> Result<()> {
        let state = self.state.lock();
        if !state.is_enabled {
            return_errno_with_message!(Errno::EINVAL, "the DRM CRTC is inactive");
        }
        if state.pending_flip.is_some() {
            return_errno_with_message!(Errno::EBUSY, "a DRM page flip is pending");
        }
        Ok(())
    }

    /// Completes a software scanout update at the next software vblank boundary.
    ///
    /// The caller serializes this with modesets and other flips. The framebuffer
    /// copy must have succeeded before this method is called.
    pub(crate) fn submit_page_flip(
        &self,
        user_data: u64,
        crtc_id: u32,
        reservation: Option<DrmEventReservation>,
    ) {
        let mut state = self.state.lock();
        let target = state.sequence.wrapping_add(1);
        state.pending_flip = Some(target);
        if let Some(reservation) = reservation {
            let mut request = DrmVblankRequest::new(target, user_data, crtc_id, reservation);
            request.event_type = DrmEventType::FlipComplete;
            state.requests.push_back(request);
        }
    }

    /// Submits a request while vblank is enabled, completing reached targets immediately.
    ///
    /// Returns the current sequence for immediate completion, or the queued target sequence.
    pub(crate) fn submit_request(&self, request: DrmVblankRequest) -> Result<u64> {
        let (sequence, timestamp) = {
            let mut state = self.state.lock();
            if !state.is_enabled {
                return_errno_with_message!(Errno::EINVAL, "the DRM vblank counter is disabled");
            }

            if !state.has_reached_sequence(request.target_sequence) {
                let target_sequence = request.target_sequence;
                state.requests.push_back(request);
                return Ok(target_sequence);
            }

            (state.sequence, state.timestamp)
        };

        // Delivers outside the state lock to keep event queue locking and reader
        // notification out of the critical section.
        // Concurrent responses may be delivered out of sequence order.
        let completed_request = DrmVblankCompletion::new(request, sequence, timestamp);
        completed_request.deliver();

        Ok(sequence)
    }

    /// Waits for the target, CRTC shutdown, a signal, or timeout.
    ///
    /// The caller must not hold the CRTC state lock.
    pub(crate) fn wait_target_sequence(&self, target_sequence: u64) -> Result<DrmVblankWaitResult> {
        {
            let state = self.state.lock();
            if !state.is_enabled {
                return_errno_with_message!(Errno::EINVAL, "the DRM vblank counter is disabled");
            }
            if state.has_reached_sequence(target_sequence) {
                return Ok(DrmVblankWaitResult::Finished {
                    sequence: state.sequence,
                    timestamp: state.timestamp,
                });
            }
        }

        // Times out after 3 seconds.
        // Reference: <https://elixir.bootlin.com/linux/v6.17/source/drivers/gpu/drm/drm_vblank.c#L1841-L1844>.
        const WAIT_TIMEOUT: Duration = Duration::from_secs(3);
        let result = self.wait_queue.pause_until_or_timeout(
            || {
                let state = self.state.lock();
                (state.has_reached_sequence(target_sequence) || !state.is_enabled).then_some(
                    DrmVblankWaitResult::Finished {
                        sequence: state.sequence,
                        timestamp: state.timestamp,
                    },
                )
            },
            Some(&WAIT_TIMEOUT),
        );

        match result {
            Err(error) if error.error() == Errno::ETIME => {
                // Preserves a coherent snapshot for the ioctl's timeout reply.
                let state = self.state.lock();
                Ok(DrmVblankWaitResult::TimedOut {
                    sequence: state.sequence,
                    timestamp: state.timestamp,
                })
            }
            result => result,
        }
    }

    /// Reports vblanks and completes reached requests.
    ///
    /// `timestamp` is the monotonic time of the last reported vblank.
    /// Ignores zero increments and reports received while disabled.
    /// Must be called from task context because event handling may block.
    pub fn handle_vblank(&self, timestamp: Duration, sequence_delta: u64) {
        if sequence_delta == 0 {
            return;
        }

        let completed_requests = {
            let mut state = self.state.lock();
            if !state.is_enabled {
                return;
            }

            state.advance(timestamp, sequence_delta);
            if state
                .pending_flip
                .is_some_and(|target| state.has_reached_sequence(target))
            {
                state.pending_flip = None;
            }

            let pending_count = state.requests.len();
            let mut completed_requests = Vec::new();
            for _ in 0..pending_count {
                let request = state.requests.pop_front().unwrap();
                if state.has_reached_sequence(request.target_sequence) {
                    completed_requests.push(DrmVblankCompletion::new(
                        request,
                        state.sequence,
                        state.timestamp,
                    ));
                } else {
                    state.requests.push_back(request);
                }
            }
            completed_requests
        };

        // Wakes synchronous ioctl callers to recheck their target sequences.
        self.wait_queue.wake_all();

        for complete_request in completed_requests {
            complete_request.deliver();
        }
    }
}

impl Debug for DrmVblank {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DrmVblank")
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

/// A synchronous vblank wait result with its sequence and timestamp.
#[derive(Debug)]
pub(crate) enum DrmVblankWaitResult {
    /// A reached target or CRTC shutdown.
    Finished { sequence: u64, timestamp: Duration },
    /// A timeout snapshot.
    TimedOut { sequence: u64, timestamp: Duration },
}

/// A CRTC's vblank snapshot and pending requests.
#[derive(Debug, Default)]
struct DrmVblankState {
    /// A flip awaiting its completion boundary, including flips without an event.
    pending_flip: Option<u64>,
    is_enabled: bool,
    /// The wrapping vblank sequence.
    sequence: u64,
    /// The monotonic timestamp of `sequence`, or zero while invalid.
    timestamp: Duration,
    /// Pending asynchronous requests.
    requests: VecDeque<DrmVblankRequest>,
}

impl DrmVblankState {
    fn has_reached_sequence(&self, target_sequence: u64) -> bool {
        self.sequence.wrapping_sub(target_sequence) <= MAX_PASSED_DISTANCE
    }

    fn advance(&mut self, timestamp: Duration, sequence_delta: u64) {
        self.sequence = self.sequence.wrapping_add(sequence_delta);
        self.timestamp = timestamp;
    }
}
