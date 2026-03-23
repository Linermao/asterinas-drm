use alloc::sync::Arc;
use core::fmt::Debug;

use crate::drm::{
    DrmDevice, DrmError,
    gem::DrmGemObject,
    ioctl::DrmModeFbDirtyCmd,
    mode_object::{DrmObject, DrmObjectCast},
};

pub trait DrmFramebuffer: Debug + Sync + Send {
    fn width(&self) -> u32;
    fn height(&self) -> u32;
    fn gem_object(&self) -> Arc<dyn DrmGemObject>;
}

impl DrmObjectCast for dyn DrmFramebuffer {
    fn cast(obj: &DrmObject) -> Option<&Arc<Self>> {
        if let DrmObject::Framebuffer(f) = obj {
            Some(f)
        } else {
            None
        }
    }
}
