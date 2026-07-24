// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;
use core::{fmt, time::Duration};

use aster_time::{Timeout, monotonic_timer_manager};
use ostd::sync::{Mutex, WaitQueue, Waiter, Waker};

use crate::DrmError;

mod fence;

pub use fence::{DrmFence, DrmFenceCallback, DrmFenceStatus};

bitflags::bitflags! {
    pub struct DrmSyncObjCreateFlags: u32 {
        const SIGNALED = 1 << 0;
    }
}

bitflags::bitflags! {
    pub struct DrmSyncObjWaitFlags: u32 {
        const WAIT_ALL = 1 << 0;
        const WAIT_FOR_SUBMIT = 1 << 1;
        const WAIT_AVAILABLE = 1 << 2;
        const WAIT_DEADLINE = 1 << 3;
    }
}

bitflags::bitflags! {
    pub struct DrmSyncObjQueryFlags: u32 {
        const LAST_SUBMITTED = 1 << 0;
    }
}

/// Selects the condition observed by a syncobj waiter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrmSyncObjWaitCondition {
    Available,
    Signaled,
}

pub struct DrmSyncObj {
    state: Mutex<DrmSyncObjState>,
    wait_queue: WaitQueue,
}

impl fmt::Debug for DrmSyncObj {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DrmSyncObj")
            .field("state", &self.state.lock())
            .finish_non_exhaustive()
    }
}

impl DrmSyncObj {
    pub fn new(signaled: bool) -> Self {
        Self {
            state: Mutex::new(DrmSyncObjState {
                fence: signaled.then(|| Arc::new(DrmFence::new(true))),
            }),
            wait_queue: WaitQueue::new(),
        }
    }

    pub fn is_submitted(&self) -> bool {
        self.state.lock().fence.is_some()
    }

    pub fn fence(&self) -> Option<Arc<DrmFence>> {
        self.fence_at(0)
    }

    /// Returns the fence representing a binary syncobj or timeline point.
    pub fn fence_at(&self, point: u64) -> Option<Arc<DrmFence>> {
        Self::find_fence(&self.state.lock(), point)
    }

    pub fn is_signaled(&self) -> bool {
        self.state
            .lock()
            .fence
            .as_ref()
            .is_some_and(|fence| fence.is_signaled())
    }

    pub fn wait(&self) {
        self.wait_point(0);
    }

    /// Waits until a timeline point becomes available.
    pub fn wait_point_available(&self, point: u64) -> Arc<DrmFence> {
        self.wait_queue
            .wait_until(|| Self::find_fence(&self.state.lock(), point))
    }

    /// Waits until a timeline point is submitted and signaled.
    pub fn wait_point(&self, point: u64) {
        self.wait_point_available(point).wait();
    }

    pub fn wait_timeout(&self, timeout: Option<Duration>) -> Result<(), DrmError> {
        let deadline = timeout.map(|duration| {
            monotonic_timer_manager()
                .read_time()
                .saturating_add(duration)
        });
        self.wait_point_deadline(0, deadline)
    }

    /// Waits for a timeline point with a relative timeout.
    pub fn wait_point_timeout(
        &self,
        point: u64,
        timeout: Option<Duration>,
    ) -> Result<(), DrmError> {
        let deadline = timeout.map(|duration| {
            monotonic_timer_manager()
                .read_time()
                .saturating_add(duration)
        });
        self.wait_point_deadline(point, deadline)
    }

    /// Waits until a timeline point becomes available with a relative timeout.
    pub fn wait_point_available_timeout(
        &self,
        point: u64,
        timeout: Option<Duration>,
    ) -> Result<Arc<DrmFence>, DrmError> {
        let deadline = timeout.map(|duration| {
            monotonic_timer_manager()
                .read_time()
                .saturating_add(duration)
        });
        self.wait_fence_submitted_deadline(point, deadline)
    }

    fn wait_point_deadline(&self, point: u64, deadline: Option<Duration>) -> Result<(), DrmError> {
        let fence = self.wait_fence_submitted_deadline(point, deadline)?;
        fence.wait_deadline(deadline)
    }

    fn wait_fence_submitted_deadline(
        &self,
        point: u64,
        deadline: Option<Duration>,
    ) -> Result<Arc<DrmFence>, DrmError> {
        if let Some(fence) = self.fence_at(point) {
            return Ok(fence);
        }
        if deadline.is_some_and(|deadline| monotonic_timer_manager().read_time() >= deadline) {
            return Err(DrmError::Busy);
        }

        let (waiter, _) = Waiter::new_pair();
        let timer = deadline.map(|deadline| {
            let waker = waiter.waker();
            let timer = monotonic_timer_manager().create_timer(move |_guard| {
                waker.wake_up();
            });
            timer.lock().set_timeout(Timeout::When(deadline));
            timer
        });

        let result = waiter.wait_until_or_cancelled(
            || {
                self.fence_at_and_register_waiter(
                    point,
                    DrmSyncObjWaitCondition::Available,
                    waiter.waker(),
                )
            },
            || {
                if timer
                    .as_ref()
                    .is_some_and(|timer| timer.lock().remain() == Duration::ZERO)
                {
                    return Err(DrmError::Busy);
                }

                Ok(())
            },
        );

        if let Some(timer) = timer
            && !result.as_ref().is_err_and(|err| *err == DrmError::Busy)
        {
            timer.lock().cancel();
        }

        result
    }

    pub fn reset(&self) {
        self.state.lock().fence = None;
        self.wait_queue.wake_all();
    }

    /// Replaces a binary syncobj payload with a signaled stub fence.
    pub fn signal(&self) {
        self.set_fence(Arc::new(DrmFence::new(true)));
    }

    pub fn set_fence(&self, fence: Arc<DrmFence>) {
        self.state.lock().fence = Some(fence);
        self.wait_queue.wake_all();
    }

    /// Adds a fence at a timeline point, or replaces the binary payload at point zero.
    pub fn add_point(&self, point: u64, fence: Arc<DrmFence>) -> Result<(), DrmError> {
        if point == 0 {
            self.set_fence(fence);
            return Ok(());
        }

        self.add_timeline_point(point, fence)
    }

    fn add_timeline_point(&self, point: u64, fence: Arc<DrmFence>) -> Result<(), DrmError> {
        let contained = fence.as_chain_dependency();
        let mut state = self.state.lock();
        let chain = Arc::new(DrmFence::new_chain(state.fence.clone(), contained, point)?);
        state.fence = Some(chain);
        drop(state);
        self.wait_queue.wake_all();
        Ok(())
    }

    /// Adds a signaled stub fence at a timeline point.
    pub fn signal_point(&self, point: u64) -> Result<(), DrmError> {
        self.add_timeline_point(point, Arc::new(DrmFence::new(true)))
    }

    /// Returns the last point submitted to an ordered timeline.
    pub fn last_submitted_point(&self) -> u64 {
        self.fence_at(0)
            .and_then(|fence| fence.sequence_number())
            .unwrap_or(0)
    }

    /// Returns the latest continuously signaled point of the current timeline context.
    pub fn last_signaled_point(&self) -> u64 {
        let Some(head) = self.fence_at(0) else {
            return 0;
        };
        if head.sequence_number().is_none() {
            return 0;
        }

        let mut last = head.clone();
        let mut current = Some(head.clone());
        while let Some(fence) = current {
            if !fence.is_same_timeline(&head) {
                break;
            }

            last = fence.clone();
            current = fence.previous_fence();
        }

        if last.is_signaled() {
            last.sequence_number().unwrap_or(0)
        } else {
            last.previous_sequence_number().unwrap_or(0)
        }
    }

    pub fn register_waiter(&self, waker: Arc<Waker>) {
        self.fence_and_register_waiter(waker);
    }

    pub fn fence_and_register_waiter(&self, waker: Arc<Waker>) -> Option<Arc<DrmFence>> {
        self.fence_at_and_register_waiter(0, DrmSyncObjWaitCondition::Signaled, waker)
    }

    /// Atomically observes a point and registers for the requested future transition.
    pub fn fence_at_and_register_waiter(
        &self,
        point: u64,
        condition: DrmSyncObjWaitCondition,
        waker: Arc<Waker>,
    ) -> Option<Arc<DrmFence>> {
        let state = self.state.lock();
        let fence = Self::find_fence(&state, point);
        if fence.is_none() {
            self.wait_queue.enqueue(waker.clone());
        }
        drop(state);

        if condition == DrmSyncObjWaitCondition::Signaled
            && let Some(fence) = &fence
        {
            fence.register_waiter(waker);
        }

        fence
    }

    fn find_fence(state: &DrmSyncObjState, point: u64) -> Option<Arc<DrmFence>> {
        let fence = state.fence.as_ref()?;
        fence.find_chain_point(point)
    }
}

#[derive(Debug, Default)]
struct DrmSyncObjState {
    fence: Option<Arc<DrmFence>>,
}
