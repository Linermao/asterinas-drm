use alloc::{boxed::Box, collections::VecDeque, fmt, sync::Arc, vec::Vec};
use core::{
    fmt::Debug,
    mem::size_of,
    sync::atomic::{AtomicU32, AtomicU64, Ordering},
    time::Duration,
};

use aster_time::read_monotonic_time;
use ostd::sync::Mutex;

use crate::drm::{DrmError, mode_object::crtc::DrmCrtc};

pub const DRM_EVENT_VBLANK_LEN: u32 = 32;

#[repr(u32)]
#[derive(Debug)]
pub enum DrmVblankEvent {
    Vblank = 0x01,
    FlipComplete = 0x02,
    CrtcSequence = 0x03,
}

/// Callback trait for sending vblank events to userspace
///
/// This allows the vblank subsystem (comps/gpu) to send events
/// without depending on specific types like DrmFile.
/// Users provide closures/callbacks that implement this trait.
pub trait VblankCallback: Send + Sync {
    /// Send vblank event data to userspace
    fn send_vblank_event(&self, bytes: &[u8]);
}

// Existing structures...
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct DrmEvent {
    type_: u32,
    length: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct DrmEventVblank {
    base: DrmEvent,
    user_data: u64,
    tv_sec: u32,
    tv_usec: u32,
    sequence: u32,
    /// 0 on older kernels that do not support this
    crtc_id: u32,
}

impl DrmEventVblank {
    /// Fill in timestamp and sequence fields
    pub fn fill_timestamp(&mut self, sequence: u64, tv_sec: u32, tv_usec: u32) {
        self.sequence = sequence as u32;
        self.tv_sec = tv_sec;
        self.tv_usec = tv_usec;
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct DrmEventCrtcSequence {
    base: DrmEvent,
    user_data: u64,
    time_ns: i64,
    sequence: u64,
}

#[derive(Debug)]
pub enum DrmEventEnum {
    Vblank(DrmEventVblank),
    Sequence(DrmEventCrtcSequence),
}

pub struct DrmPendingVblankEvent {
    drm_pending_event: DrmPendingEvent,
    pipe: u32,
    sequence: u64,
    payload: DrmEventEnum,

    /// Callback to send event to userspace
    callback: Arc<dyn VblankCallback>,
}

impl fmt::Debug for DrmPendingVblankEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DrmPendingVblankEvent")
            .field("pipe", &self.pipe)
            .field("sequence", &self.sequence)
            .field("payload", &self.payload)
            .finish()
    }
}

impl DrmPendingVblankEvent {
    /// Create a new vblank event with a callback
    ///
    /// # Arguments
    /// * `callback` - Callback that will be invoked to send the event
    /// * `user_data` - User-provided data that will be returned in the event
    /// * `crtc_id` - ID of the CRTC that will generate this vblank
    pub fn new(user_data: u64, crtc_id: u32, callback: Arc<dyn VblankCallback>) -> Self {
        let drm_event = DrmEvent {
            type_: DrmVblankEvent::FlipComplete as u32,
            length: size_of::<DrmEventVblank>() as u32,
        };

        let vbl_event = DrmEventVblank {
            base: drm_event,
            user_data,
            tv_sec: 0,
            tv_usec: 0,
            sequence: 0,
            crtc_id,
        };

        Self {
            drm_pending_event: DrmPendingEvent::new(drm_event),
            pipe: 0,
            sequence: 0,
            payload: DrmEventEnum::Vblank(vbl_event),
            callback,
        }
    }

    /// Get mutable reference to the vblank event payload
    pub fn vblank_event_mut(&mut self) -> Option<&mut DrmEventVblank> {
        match &mut self.payload {
            DrmEventEnum::Vblank(ev) => Some(ev),
            _ => None,
        }
    }

    /// Send this event to userspace
    ///
    /// This is called by drm_crtc_handle_vblank when the vblank occurs.
    pub fn send(&self, sequence: u64, tv_sec: u32, tv_usec: u32) {
        // Get user_data and crtc_id from current event
        let (user_data, crtc_id) = match &self.payload {
            DrmEventEnum::Vblank(v) => (v.user_data, v.crtc_id),
            _ => (0, 0),
        };

        // Manually construct event bytes with updated timestamp/sequence
        let mut bytes = Vec::with_capacity(size_of::<DrmEventVblank>());

        // type: DRM_EVENT_VBLANK
        bytes.extend_from_slice(&self.drm_pending_event.drm_event.type_.to_ne_bytes());
        // length
        bytes.extend_from_slice(&(size_of::<DrmEventVblank>() as u32).to_ne_bytes());
        // user_data
        bytes.extend_from_slice(&user_data.to_ne_bytes());
        // tv_sec
        bytes.extend_from_slice(&tv_sec.to_ne_bytes());
        // tv_usec
        bytes.extend_from_slice(&tv_usec.to_ne_bytes());
        // sequence
        bytes.extend_from_slice(&(sequence as u32).to_ne_bytes());
        // crtc_id
        bytes.extend_from_slice(&crtc_id.to_ne_bytes());

        // Invoke callback to send the event
        self.callback.send_vblank_event(&bytes);
    }

    /// Get the CRTC ID this event is associated with
    pub fn crtc_id(&self) -> u32 {
        match &self.payload {
            DrmEventEnum::Vblank(v) => v.crtc_id,
            DrmEventEnum::Sequence(_) => 0,
        }
    }

    /// Serialize the event to bytes for delivery to userspace via read().
    ///
    /// Returns a byte array containing the event data in the format
    /// expected by the Linux DRM ABI.
    pub fn to_bytes(&self) -> Vec<u8> {
        match &self.payload {
            DrmEventEnum::Vblank(vblank) => {
                let mut bytes = Vec::with_capacity(size_of::<DrmEventVblank>());

                // Manually serialize each field (safe, no unsafe)
                bytes.extend_from_slice(&vblank.base.type_.to_ne_bytes());
                bytes.extend_from_slice(&vblank.base.length.to_ne_bytes());
                bytes.extend_from_slice(&vblank.user_data.to_ne_bytes());
                bytes.extend_from_slice(&vblank.tv_sec.to_ne_bytes());
                bytes.extend_from_slice(&vblank.tv_usec.to_ne_bytes());
                bytes.extend_from_slice(&vblank.sequence.to_ne_bytes());
                bytes.extend_from_slice(&vblank.crtc_id.to_ne_bytes());

                bytes
            }
            DrmEventEnum::Sequence(seq) => {
                let mut bytes = Vec::with_capacity(size_of::<DrmEventCrtcSequence>());

                // Manually serialize each field (safe, no unsafe)
                bytes.extend_from_slice(&seq.base.type_.to_ne_bytes());
                bytes.extend_from_slice(&seq.base.length.to_ne_bytes());
                bytes.extend_from_slice(&seq.user_data.to_ne_bytes());
                bytes.extend_from_slice(&seq.time_ns.to_ne_bytes());
                bytes.extend_from_slice(&seq.sequence.to_ne_bytes());

                bytes
            }
        }
    }
}

#[derive(Debug)]
pub struct DrmPendingEvent {
    drm_event: DrmEvent,
}

impl DrmPendingEvent {
    pub fn new(drm_event: DrmEvent) -> Self {
        Self { drm_event }
    }
}

/// Vblank state for a CRTC
///
/// This is created on-demand when vblank functionality is first needed.
/// It corresponds to Linux's `struct drm_vblank_crtc`.
#[derive(Debug)]
pub struct DrmVblankState {
    /// Vblank counter (starts at 0, increments on each vblank)
    counter: AtomicU64,

    /// Reference count of vblank users
    /// When 0 -> 1: enable hardware interrupt
    /// When 1 -> 0: disable hardware interrupt
    refcount: AtomicU32,

    /// Timestamp of last vblank
    last_time: Mutex<Duration>,

    /// Pending events waiting to be sent on next vblank
    pending_events: Mutex<VecDeque<DrmPendingVblankEvent>>,
}

impl DrmVblankState {
    /// Create a new vblank state
    pub fn new() -> Self {
        Self {
            counter: AtomicU64::new(0),
            refcount: AtomicU32::new(0),
            last_time: Mutex::new(Duration::from_secs(0)),
            pending_events: Mutex::new(VecDeque::new()),
        }
    }

    /// Get current counter value
    pub fn counter(&self) -> u64 {
        self.counter.load(Ordering::SeqCst)
    }

    /// Increment counter and return new value
    pub fn increment(&self) -> u64 {
        self.counter.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Get reference count
    pub fn refcount(&self) -> u32 {
        self.refcount.load(Ordering::SeqCst)
    }

    /// Increment refcount (enable vblank)
    ///
    /// Returns true if this was the first ref (need to enable HW interrupt)
    pub fn get(&self) -> bool {
        self.refcount.fetch_add(1, Ordering::SeqCst) == 0
    }

    /// Decrement refcount (disable vblank)
    ///
    /// Returns true if this was the last ref (need to disable HW interrupt)
    pub fn put(&self) -> bool {
        let prev = self.refcount.fetch_sub(1, Ordering::SeqCst);
        debug_assert!(prev > 0, "vblank refcount underflow");
        prev == 1
    }

    /// Queue a pending event to be sent on next vblank
    pub fn queue_event(&self, event: DrmPendingVblankEvent) {
        self.pending_events.lock().push_back(event);
    }

    /// Take all pending events (for processing in vblank handler)
    ///
    /// This drains the queue and returns all pending events.
    /// Should be called by drm_crtc_handle_vblank().
    pub fn take_pending_events(&self) -> Vec<DrmPendingVblankEvent> {
        let mut queue = self.pending_events.lock();
        queue.drain(..).collect()
    }

    /// Update last vblank timestamp
    pub fn update_time(&self, time: Duration) {
        *self.last_time.lock() = time;
    }

    /// Get last vblank timestamp
    pub fn last_time(&self) -> Duration {
        *self.last_time.lock()
    }
}

pub struct PageFlipEvent {
    user_data: u64,
    vblank_callback: Arc<dyn VblankCallback>,
}

impl PageFlipEvent {
    pub fn new(user_data: u64, vblank_callback: Arc<dyn VblankCallback>) -> Self {
        Self {
            user_data,
            vblank_callback,
        }
    }

    pub fn user_data(&self) -> u64 {
        self.user_data
    }

    pub fn vblank_callback(&self) -> Arc<dyn VblankCallback> {
        self.vblank_callback.clone()
    }
}
