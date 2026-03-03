use alloc::sync::Arc;

use aster_time::read_monotonic_time;
use ostd::early_println;

use crate::drm::{
    DrmError,
    device::DrmDevice,
    mode_config::{crtc::DrmCrtc, framebuffer::DrmFramebuffer},
    vblank::DrmPendingVblankEvent,
};

fn page_flip_common(crtc: Arc<DrmCrtc>, event: DrmPendingVblankEvent) {
    // Queue event in CRTC's vblank state
    early_println!("[kernel] page_flip_common");
    let vblank = crtc.vblank_state();
    vblank.lock().queue_event(event);
}

fn drm_atomic_nonblocking_commit(device: Arc<DrmDevice>) -> Result<(), DrmError> {
    let mode_config = device.resources().lock();
    mode_config.funcs.atomic_commit(true)
}

pub fn drm_atomic_helper_page_flip(
    device: Arc<DrmDevice>,
    crtc: Arc<DrmCrtc>,
    fb: Arc<DrmFramebuffer>,
    event: Option<DrmPendingVblankEvent>,
    flags: u32,
    target: Option<u32>,
) -> Result<(), DrmError> {
    if let Some(event) = event {
        page_flip_common(crtc, event);
    }
    drm_atomic_nonblocking_commit(device)
}

/// Handle a vblank interrupt for a CRTC
///
/// This function should be called by the driver's vblank interrupt handler.
/// It performs the following:
/// 1. Increments the vblank counter
/// 2. Records the current time
/// 3. Processes all pending events (page_flip, vblank wait)
/// 4. Sends events to userspace via the stored sender
///
/// # Arguments
/// * `crtc` - The CRTC that generated the vblank interrupt
///
/// # Returns
/// * `Ok(())` - Vblank handled successfully
/// * `Err(DrmError)` - Error processing vblank
pub fn drm_crtc_handle_vblank(crtc: Arc<DrmCrtc>) -> Result<(), DrmError> {
    early_println!("[kernel] handle vblank");
    let vblank_state = crtc.vblank_state();
    let vblank = vblank_state.lock();

    // 1. Increment vblank counter
    let sequence = vblank.increment();

    // 2. Update timestamp
    let timestamp = read_monotonic_time(); // TODO: Get actual monotonic time
    vblank.update_time(timestamp);

    // 3. Take all pending events
    let pending_events = vblank.take_pending_events();

    // 4. Send each event
    for event in pending_events {
        event.send(
            sequence,
            timestamp.as_secs() as u32,
            timestamp.subsec_micros(),
        );
    }

    // TODO: Wake up any DRM_IOCTL_WAIT_VBLANK waiters

    Ok(())
}
