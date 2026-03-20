use alloc::sync::Arc;
use core::fmt::Debug;

use ostd::sync::Mutex;

use crate::drm::{mode_config::ObjectId, mode_object::{DrmObject, DrmObjectCast, property::PropertyObject}};

pub mod property;

#[repr(u32)]
#[derive(Debug, Clone, Copy)]
pub enum DrmPlaneType {
    Overlay = 0,
    Primary = 1,
    Cursor = 2,
}

#[derive(Debug)]
pub struct PlaneState {
    src_x: u32,
    src_y: u32,
    src_w: u32,
    src_h: u32,

    crtc_x: u32,
    crtc_y: u32,
    crtc_w: u32,
    crtc_h: u32,

    possible_crtcs: u32,
    properties: PropertyObject,

    fb_id: Option<ObjectId>,
    crtc_id: Option<ObjectId>,
}

impl PlaneState {
    pub fn new(properties: PropertyObject) -> Self {
        Self {
            src_x: 0,
            src_y: 0,
            src_w: 0,
            src_h: 0,
            crtc_x: 0,
            crtc_y: 0,
            crtc_w: 0,
            crtc_h: 0,
            possible_crtcs: 0,
            properties,
            fb_id: None,
            crtc_id: None,
        }
    }

    pub fn set_possible_crtcs(&mut self, crtc_indices: &[usize]) {
        for &idx in crtc_indices {
            self.possible_crtcs |= 1 << idx;
        }
    }
}

pub trait DrmPlane: Debug + Send + Sync {
    fn state(&self) -> &Mutex<PlaneState>;
}

impl dyn DrmPlane {
    pub fn src_x(&self) -> u32 {
        self.state().lock().src_x
    }
    pub fn src_y(&self) -> u32 {
        self.state().lock().src_y
    }
    pub fn src_w(&self) -> u32 {
        self.state().lock().src_w
    }
    pub fn src_h(&self) -> u32 {
        self.state().lock().src_h
    }
    pub fn crtc_x(&self) -> u32 {
        self.state().lock().crtc_x
    }
    pub fn crtc_y(&self) -> u32 {
        self.state().lock().crtc_y
    }
    pub fn crtc_w(&self) -> u32 {
        self.state().lock().crtc_w
    }
    pub fn crtc_h(&self) -> u32 {
        self.state().lock().crtc_h
    }
    pub fn fb_id(&self) -> Option<ObjectId> {
        self.state().lock().fb_id
    }
    pub fn crtc_id(&self) -> Option<ObjectId> {
        self.state().lock().crtc_id
    }
    pub fn set_crtc_id(&self, crtc_id: Option<ObjectId>) {
        self.state().lock().crtc_id = crtc_id;
    }
    pub fn set_fb_id(&self, fb_id: Option<ObjectId>) {
        self.state().lock().fb_id = fb_id;
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