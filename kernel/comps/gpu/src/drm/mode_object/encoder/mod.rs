use alloc::sync::Arc;
use hashbrown::HashMap;
use core::fmt::Debug;

use ostd::sync::Mutex;

use crate::drm::mode_object::{DrmObject, DrmObjectCast, property::PropertyObject};

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
    fn possible_crtcs(&self) -> u32 {
        self.state().lock().possible_crtcs
    }
    fn possible_clones(&self) -> u32 {
        self.state().lock().possible_clones
    }
}

#[derive(Debug)]
pub struct EncoderState {
    possible_crtcs: u32,
    possible_clones: u32,
}

impl EncoderState {
    pub fn new() -> Self {
        Self {
            possible_crtcs: 0,
            possible_clones: 0,
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
