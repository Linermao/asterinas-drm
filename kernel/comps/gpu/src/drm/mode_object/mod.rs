use alloc::sync::Arc;
use core::fmt::Debug;

use hashbrown::HashMap;

use crate::drm::{
    DrmError,
    mode_object::{
        connector::DrmConnector,
        crtc::DrmCrtc,
        encoder::DrmEncoder,
        plane::DrmPlane,
        property::{DrmModeBlob, DrmProperty, PropertyObject},
    },
};

pub mod connector;
pub mod crtc;
pub mod encoder;
pub mod plane;
pub mod property;

#[derive(Debug, Clone)]
pub enum DrmObject {
    Plane(Arc<dyn DrmPlane>),
    Crtc(Arc<dyn DrmCrtc>),
    Encoder(Arc<dyn DrmEncoder>),
    Connector(Arc<dyn DrmConnector>),
    Property(Arc<DrmProperty>),
    Blob(Arc<DrmModeBlob>),
}

pub trait DrmObjectCast {
    fn cast(obj: &DrmObject) -> Option<&Arc<Self>>;
}

impl DrmObject {
    pub fn type_(&self) -> DrmObjectType {
        match self {
            DrmObject::Plane(_) => DrmObjectType::Plane,
            DrmObject::Crtc(_) => DrmObjectType::Crtc,
            DrmObject::Encoder(_) => DrmObjectType::Encoder,
            DrmObject::Connector(_) => DrmObjectType::Connector,
            DrmObject::Property(_) => DrmObjectType::Property,
            DrmObject::Blob(_) => DrmObjectType::Blob,
        }
    }

    pub fn count_props(&self) -> u32 {
        match self {
            DrmObject::Plane(plane) => plane.count_props(),
            DrmObject::Crtc(crtc) => crtc.count_props(),
            DrmObject::Connector(connector) => connector.count_props(),
            _ => 0,
        }
    }

    pub fn get_properties(&self) -> PropertyObject {
        match self {
            DrmObject::Plane(plane) => plane.get_properties(),
            DrmObject::Crtc(crtc) => crtc.get_properties(),
            DrmObject::Connector(connector) => connector.get_properties(),
            _ => HashMap::new(),
        }
    }
}

#[repr(u32)]
#[derive(Debug, PartialEq, Eq)]
pub enum DrmObjectType {
    Any = 0,
    Crtc = 0xCCCC_CCCC,
    Connector = 0xC0C0_C0C0,
    Encoder = 0xE0E0_E0E0,
    Mode = 0xDEDE_DEDE,
    Property = 0xB0B0_B0B0,
    Framebuffer = 0xFBFB_FBFB,
    Blob = 0xBBBB_BBBB,
    Plane = 0xEEEE_EEEE,
}

impl TryFrom<u32> for DrmObjectType {
    type Error = DrmError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Any),
            0xCCCC_CCCC => Ok(Self::Crtc),
            0xC0C0_C0C0 => Ok(Self::Connector),
            0xE0E0_E0E0 => Ok(Self::Encoder),
            0xDEDE_DEDE => Ok(Self::Mode),
            0xB0B0_B0B0 => Ok(Self::Property),
            0xFBFB_FBFB => Ok(Self::Framebuffer),
            0xBBBB_BBBB => Ok(Self::Blob),
            0xEEEE_EEEE => Ok(Self::Plane),
            _ => Err(DrmError::Invalid),
        }
    }
}
