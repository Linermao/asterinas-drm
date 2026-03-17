use alloc::{sync::Arc, vec::Vec};
use core::fmt::Debug;

use ostd::sync::Mutex;

use crate::drm::{
    DrmDevice, DrmError, drm_modes::DrmDisplayMode, mode_object::{
        DrmObject, DrmObjectCast, connector::DrmConnector, framebuffer::DrmFramebuffer,
        plane::DrmPlane, property::PropertyObject,
    }
};

pub mod property;

pub trait DrmCrtc: Debug + Send + Sync {
    fn state(&self) -> &Mutex<CrtcState>;
    fn primary_plane(&self) -> &Arc<dyn DrmPlane>;
    fn cursor_plane(&self) -> &Option<Arc<dyn DrmPlane>>;
    fn set_config(
        &self,
        x: u32,
        y: u32,
        fb: Arc<dyn DrmFramebuffer>,
        connectors: Vec<Arc<dyn DrmConnector>>,
        dev: Arc<dyn DrmDevice>,
    ) -> Result<(), DrmError>;
}

impl dyn DrmCrtc {
    pub fn gamma_size(&self) -> u32 {
        self.state().lock().gamma_size
    }

    pub fn enable(&self) -> bool {
        self.state().lock().enable
    }

    pub fn set_display_mode(&self, display_mode: DrmDisplayMode) {
        self.state().lock().display_mode = Some(display_mode);
    }

    pub fn display_mode(&self) -> Option<DrmDisplayMode> {
        self.state().lock().display_mode
    }

    pub fn count_props(&self) -> u32 {
        self.state().lock().properties.iter().count() as u32
    }

    pub fn get_properties(&self) -> PropertyObject {
        self.state().lock().properties.clone()
    }

    pub fn active(&self) -> bool {
        self.state().lock().active
    }

    pub fn set_active(&self, active: bool) {
        self.state().lock().active = active;
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
    active: bool,
}

impl CrtcState {
    pub fn new(properties: PropertyObject) -> Self {
        Self {
            properties,
            display_mode: None,
            gamma_size: 0,
            enable: false,
            active: false,
        }
    }
}
