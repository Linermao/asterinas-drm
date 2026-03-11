use alloc::sync::Arc;
use core::fmt::Debug;

use crate::drm::{gem::DrmGemObject, mode_object::{DrmObject, DrmObjectCast}};

pub trait DrmFramebuffer: Debug + Sync + Send {
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