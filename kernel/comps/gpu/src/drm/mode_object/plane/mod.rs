use alloc::sync::Arc;
use core::fmt::Debug;

use ostd::sync::Mutex;

use crate::drm::mode_object::{DrmObject, DrmObjectCast, property::PropertyObject};

pub mod property;

pub trait DrmPlane: Debug + Send + Sync {
    fn state(&self) -> &Mutex<PlaneState>;
}

impl dyn DrmPlane {
    pub fn fb_id(&self) -> u32 {
        self.state().lock().fb_id
    }

    pub fn src_x(&self) -> u32 {
        self.state().lock().src_x
    }

    pub fn src_y(&self) -> u32 {
        self.state().lock().src_y
    }

    pub fn possible_crtcs(&self) -> u32 {
        self.state().lock().possible_crtcs
    }

    pub fn count_props(&self) -> u32 {
        self.state().lock().properties.iter().count() as u32
    }

    pub fn get_properties(&self) -> PropertyObject {
        self.state().lock().properties.clone()
    }
}

impl DrmObjectCast for dyn DrmPlane {
    fn cast(obj: &DrmObject) -> Option<&Arc<Self>> {
        if let DrmObject::Plane(p) = obj {
            Some(p)
        } else {
            None
        }
    }
}

#[derive(Debug)]
pub struct PlaneState {
    src_x: u32,
    src_y: u32,
    fb_id: u32,
    possible_crtcs: u32,
    properties: PropertyObject,
}

impl PlaneState {
    pub fn new(properties: PropertyObject) -> Self {
        Self {
            src_x: 0,
            src_y: 0,
            fb_id: 0,
            possible_crtcs: 0,
            properties,
        }
    }

    pub fn set_possible_crtcs(&mut self, crtc_indices: &[usize]) {
        for &idx in crtc_indices {
            self.possible_crtcs |= 1 << idx;
        }
    }
}
