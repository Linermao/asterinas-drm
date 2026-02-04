use core::{any::Any, fmt::Debug};

use crate::drm::{DrmError, mode_config::framebuffer::DrmFramebuffer};

// TODO
pub trait FramebufferFuncs: Debug + Any + Sync + Send {
    // fn destroy(&self, fb: DrmFramebuffer);
    // fn create_handle(&self, fb: DrmFramebuffer) -> Result<u32, DrmError>;
    // fn dirty(&self, fb: DrmFramebuffer) -> Result<(), DrmError>;
}
