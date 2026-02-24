use ostd::Pod;

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, Pod)]
struct DrmEvent {
    type_: u32,
    length: u32,
}

#[derive(Debug)]
pub struct DrmPendingVblankEvent {
    drm_pending_event: DrmPendingEvent,
}

#[derive(Debug)]
pub struct DrmPendingEvent {
    drm_event: DrmEvent,
}
