use alloc::{boxed::Box, sync::Arc};

use crate::drm::{
    DrmError,
    device::DrmDevice,
    mode_config::framebuffer::{DrmFramebuffer, funcs::FramebufferFuncs},
};

#[derive(Debug)]
struct GemFbFuncsDirtyFb {}

impl FramebufferFuncs for GemFbFuncsDirtyFb {}

pub fn gem_fb_create_with_dirty(dev: &DrmDevice) -> Result<DrmFramebuffer, DrmError> {
    // validate
    // lookup GEM BO
    // refcount
    // create framebuffer

    // let fb = DrmFramebuffer::new(
    //     width,
    //     height,
    //     pitch,
    //     bpp,
    //     gem_obj,
    //     Box::new(GemFbFuncsDirtyFb {}),
    // );

    todo!()
}
