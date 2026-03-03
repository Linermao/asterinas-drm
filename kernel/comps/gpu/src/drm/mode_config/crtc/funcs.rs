use alloc::sync::Arc;
use core::{any::Any, fmt::Debug};

use crate::drm::{
    DrmError,
    device::DrmDevice,
    mode_config::{crtc::DrmCrtc, framebuffer::DrmFramebuffer},
    vblank::DrmPendingVblankEvent,
};

// TODO
pub trait CrtcFuncs: Debug + Any + Sync + Send {
    fn page_flip(
        &self,
        device: Arc<DrmDevice>,
        crtc: Arc<DrmCrtc>,
        fb: Arc<DrmFramebuffer>,
        event: Option<DrmPendingVblankEvent>,
        flags: u32,
        target: Option<u32>,
    ) -> Result<(), DrmError>;

    /// Enable hardware vblank interrupt
    fn enable_vblank(&self, crtc: Arc<DrmCrtc>) -> Result<(), DrmError>;

    /// Disable hardware vblank interrupt
    fn disable_vblank(&self, crtc: Arc<DrmCrtc>) -> Result<(), DrmError>;
}
