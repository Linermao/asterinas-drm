use alloc::sync::Arc;
use core::{any::Any, fmt::Debug};

use crate::drm::{
    DrmError, device::DrmDevice, gem::DrmGemObject, mode_config::framebuffer::DrmFramebuffer,
};

pub trait ModeConfigFuncs: Debug + Any + Sync + Send {
    fn create_framebuffer(
        &self,
        width: u32,
        height: u32,
        pitch: u32,
        bpp: u32,
        gem_obj: Arc<DrmGemObject>,
    ) -> Result<DrmFramebuffer, DrmError>;

    fn atomic_commit(&self, nonblock: bool) -> Result<(), DrmError>;
    fn atomic_commit_tail(&self) -> Result<(), DrmError>;
}

pub fn commit_tail(device: Arc<DrmDevice>) {
    let mode_config = device.resources().lock();
    mode_config.funcs.atomic_commit_tail();
}

pub fn drm_atomic_helper_swap_state(stall: bool) -> Result<(), DrmError> {
    Ok(())
}

pub fn drm_atomic_helper_commit(nonblock: bool) -> Result<(), DrmError> {
    // software commit
    drm_atomic_helper_swap_state(true)?;
    // commit_tail(device);

    Ok(())
}
