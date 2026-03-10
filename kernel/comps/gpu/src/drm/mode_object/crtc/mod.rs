use alloc::sync::Arc;
use core::fmt::Debug;

use ostd::sync::Mutex;

use crate::drm::{
    drm_modes::DrmDisplayMode,
    mode_object::{DrmObject, DrmObjectCast, plane::DrmPlane, property::PropertyObject},
};

pub mod property;

pub trait DrmCrtc: Debug + Send + Sync {
    fn state(&self) -> &Mutex<CrtcState>;

    fn primary_plane(&self) -> Arc<dyn DrmPlane>;
    fn cursor_plane(&self) -> Option<Arc<dyn DrmPlane>>;

    fn gamma_size(&self) -> u32 {
        self.state().lock().gamma_size
    }

    fn enable(&self) -> bool {
        self.state().lock().enable
    }

    fn display_mode(&self) -> Option<DrmDisplayMode> {
        self.state().lock().display_mode
    }

    fn count_props(&self) -> u32 {
        self.state().lock().properties.iter().count() as u32
    }

    fn get_properties(&self) -> PropertyObject {
        self.state().lock().properties.clone()
    }
}

impl DrmObjectCast for dyn DrmCrtc {
    fn cast(obj: &DrmObject) -> Option<&Arc<Self>> {
        if let DrmObject::Crtc(c) = obj {
            Some(c)
        } else {
            None
        }
    }
}

#[derive(Debug)]
pub struct CrtcState {
    properties: PropertyObject,
    display_mode: Option<DrmDisplayMode>,
    gamma_size: u32,
    enable: bool,
}

impl CrtcState {
    pub fn new(properties: PropertyObject) -> Self {
        Self {
            properties,
            display_mode: None,
            gamma_size: 0,
            enable: false,
        }
    }
}
