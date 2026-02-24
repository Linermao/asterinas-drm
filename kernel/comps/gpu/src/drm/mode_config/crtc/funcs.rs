use alloc::sync::Arc;
use core::{any::Any, fmt::Debug};

use crate::drm::{DrmError, mode_config::framebuffer::DrmFramebuffer};

// TODO
pub trait CrtcFuncs: Debug + Any + Sync + Send {
    fn page_flip(&self, fb: Arc<DrmFramebuffer>) -> Result<(), DrmError>;
    fn page_flip_target(&self, fb: Arc<DrmFramebuffer>) -> Result<(), DrmError>;
}
