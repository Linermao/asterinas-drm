use alloc::sync::Arc;
use core::{any::Any, fmt::Debug};

use crate::drm::{DrmError, gem::DrmGemObject, mode_config::framebuffer::DrmFramebuffer};

pub trait ModeConfigFuncs: Debug + Any + Sync + Send {
    fn create_framebuffer(
        &self,
        width: u32,
        height: u32,
        pitch: u32,
        bpp: u32,
        gem_obj: Arc<DrmGemObject>,
    ) -> Result<DrmFramebuffer, DrmError>;
}
