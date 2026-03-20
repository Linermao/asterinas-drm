use alloc::sync::Arc;
use core::fmt::Debug;

use ostd::sync::Mutex;

use crate::drm::{mode_config::ObjectId, mode_object::{DrmObject, DrmObjectCast}};

#[repr(u32)]
#[derive(Debug, Clone, Copy)]
pub enum EncoderType {
    NONE = 0,
    DAC = 1,
    TMDS = 2,
    LVDS = 3,
    TVDAC = 4,
    VIRTUAL = 5,
    DSI = 6,
    DPMST = 7,
    DPI = 8,
}

pub trait DrmEncoder: Debug + Send + Sync {
    fn type_(&self) -> EncoderType;
    fn state(&self) -> &Mutex<EncoderState>;
}

impl dyn DrmEncoder {
    pub fn possible_crtcs(&self) -> u32 {
        self.state().lock().possible_crtcs
    }
    pub fn possible_clones(&self) -> u32 {
        self.state().lock().possible_clones
    }

    pub fn crtc_id(&self) -> Option<ObjectId> {
        self.state().lock().crtc_id
    }

    pub fn set_crtc_id(&self, crtc_id: Option<ObjectId>) {
        self.state().lock().crtc_id = crtc_id;
    }
}

#[derive(Debug)]
pub struct EncoderState {
    possible_crtcs: u32,
    possible_clones: u32,
    crtc_id: Option<ObjectId>,
}

impl EncoderState {
    pub fn new() -> Self {
        Self {
            possible_crtcs: 0,
            possible_clones: 0,
            crtc_id: None
        }
    }

    pub fn set_possible_crtcs(&mut self, crtc_indices: &[usize]) {
        for &idx in crtc_indices {
            self.possible_crtcs |= 1 << idx;
        }
    }

    pub fn set_possible_clones(&mut self, crtc_indices: &[usize]) {
        for &idx in crtc_indices {
            self.possible_clones |= 1 << idx;
        }
    }
}

impl DrmObjectCast for dyn DrmEncoder {
    fn cast(obj: &DrmObject) -> Option<&Arc<Self>> {
        if let DrmObject::Encoder(e) = obj {
            Some(e)
        } else {
            None
        }
    }
}
