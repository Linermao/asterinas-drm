use alloc::{boxed::Box, sync::Arc};

use crate::drm::{
    DrmError,
    gem::DrmGemObject,
    mode_config::framebuffer::{DrmFramebuffer, funcs::FramebufferFuncs},
};

#[derive(Debug)]
struct GemFbFuncsDirtyFb {}

impl FramebufferFuncs for GemFbFuncsDirtyFb {}

pub fn drm_gem_fb_create_with_dirty(
    width: u32,
    height: u32,
    pitch: u32,
    bpp: u32,
    gem_obj: Arc<DrmGemObject>,
) -> Result<DrmFramebuffer, DrmError> {
    // validate
    // lookup GEM BO
    // refcount
    // create framebuffer

    Ok(DrmFramebuffer::new(
        width,
        height,
        pitch,
        bpp,
        gem_obj,
        Box::new(GemFbFuncsDirtyFb {}),
    ))
}
