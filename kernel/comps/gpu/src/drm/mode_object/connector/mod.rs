use alloc::{sync::Arc, vec::Vec};
use core::fmt::Debug;

use ostd::sync::Mutex;

use crate::drm::{
    DrmError,
    drm_modes::{DrmDisplayInfo, DrmDisplayMode},
    mode_object::{DrmObject, DrmObjectCast, property::PropertyObject},
};

pub mod property;

#[repr(u32)]
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub enum ConnectorType {
    UNKNOWN = 0,
    VGA = 1,
    DVII = 2,
    DVID = 3,
    DVIA = 4,
    COMPOSITE = 5,
    SVIDEO = 6,
    LVDS = 7,
    COMPONENT = 8,
    _9PINDIN = 9,
    DISPLAYPORT = 10,
    HDMIA = 11,
    HDMIB = 12,
    TV = 13,
    EDP = 14,
    VIRTUAL = 15,
    DSI = 16,
    DPI = 17,
    WRITEBACK = 18,
    SPI = 19,
    USB = 20,
}

#[repr(u32)]
#[derive(Debug, Clone, Copy)]
pub enum ConnectorStatus {
    // DRM_MODE_CONNECTED
    Connected = 1,
    // DRM_MODE_DISCONNECTED
    Disconnected = 2,
    // DRM_MODE_UNKNOWNCONNECTION
    Unknownconnection = 3,
}

pub trait DrmConnector: Debug + Send + Sync {
    fn type_(&self) -> ConnectorType;

    fn type_id_(&self) -> u32;

    fn state(&self) -> &Mutex<ConnectorState>;

    fn fill_modes(&self) -> Result<(), DrmError>;

    fn get_modes(&self) -> Result<(), DrmError>;

    fn detect(&self) -> Result<ConnectorStatus, DrmError>;
}

impl dyn DrmConnector {
    pub fn possible_encoders(&self) -> u32 {
        self.state().lock().possible_encoders
    }

    pub fn count_encoders(&self) -> u32 {
        self.state().lock().possible_encoders.count_ones()
    }

    pub fn count_modes(&self) -> u32 {
        self.state().lock().display_modes.iter().count() as u32
    }

    pub fn count_props(&self) -> u32 {
        self.state().lock().properties.iter().count() as u32
    }

    pub fn modes(&self) -> Vec<DrmDisplayMode> {
        self.state().lock().display_modes.clone()
    }

    pub fn get_properties(&self) -> PropertyObject {
        self.state().lock().properties.clone()
    }

    pub fn status(&self) -> ConnectorStatus {
        self.state().lock().status
    }

    pub fn mm_width(&self) -> u32 {
        self.state().lock().display_info.mm_width()
    }

    pub fn mm_height(&self) -> u32 {
        self.state().lock().display_info.mm_height()
    }

    pub fn subpixel(&self) -> u32 {
        self.state().lock().display_info.subpixel_order()
    }
}

#[derive(Debug)]
pub struct ConnectorState {
    status: ConnectorStatus,
    properties: PropertyObject,
    display_modes: Vec<DrmDisplayMode>,
    display_info: DrmDisplayInfo,
    possible_encoders: u32,
}

impl ConnectorState {
    pub fn new(properties: PropertyObject) -> Self {
        Self {
            status: ConnectorStatus::Unknownconnection,
            properties,
            display_modes: Vec::new(),
            display_info: DrmDisplayInfo::default(),
            possible_encoders: 0,
        }
    }

    pub fn set_possible_encoders(&mut self, encoder_indices: &[usize]) {
        for &idx in encoder_indices {
            self.possible_encoders |= 1 << idx;
        }
    }

    pub fn set_status(&mut self, status: ConnectorStatus) {
        self.status = status;
    }

    pub fn set_modes(&mut self, modes: &[DrmDisplayMode]) {
        self.display_modes.clear();

        for &mode in modes {
            self.display_modes.push(mode);
        }
    }
}

impl DrmObjectCast for dyn DrmConnector {
    fn cast(obj: &DrmObject) -> Option<&Arc<Self>> {
        if let DrmObject::Connector(c) = obj {
            Some(c)
        } else {
            None
        }
    }
}
