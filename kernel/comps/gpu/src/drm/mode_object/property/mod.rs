use alloc::{
    sync::Arc,
    vec::Vec,
};
use hashbrown::HashMap;
use core::{any::Any, fmt::Debug};

use crate::drm::mode_object::{DrmObject, DrmObjectCast};

pub const DRM_PROP_NAME_LEN: usize = 32;
pub type PropertyObject = HashMap<u32, u64>;

fn str_to_u8_32(s: &str) -> [u8; 32] {
    let mut buf = [0u8; 32];
    let bytes = s.as_bytes();
    let len = core::cmp::min(bytes.len(), 32);
    buf[..len].copy_from_slice(&bytes[..len]);
    buf
}

bitflags::bitflags! {
    pub struct PropertyFlags: u32 {
        const PENDING    = 1 << 0;
        const RANGE      = 1 << 1;
        const IMMUTABLE  = 1 << 2;
        const ENUM       = 1 << 3;
        const BLOB       = 1 << 4;
        const BITMASK    = 1 << 5;

        const ATOMIC     = 0x8000_0000;
    }
}

#[repr(u32)]
#[derive(Debug, Clone, Copy)]
pub enum DrmModeObjectType {
    Any = 0,
    Crtc = 0xCCCC_CCCC,
    Connector = 0xC0C0_C0C0,
    Encoder = 0xE0E0_E0E0,
    Mode = 0xDEDE_DEDE,
    Property = 0xB0B0_B0B0,
    FB = 0xFBFB_FBFB,
    Blob = 0xBBBB_BBBB,
    Plane = 0xEEEE_EEEE,
}

#[derive(Debug, Clone)]
pub enum PropertyKind {
    Range { min: u64, max: u64 },
    SignedRange { min: i64, max: i64 },
    Enum(Vec<PropertyEnum>),
    Bitmask(Vec<PropertyEnum>),
    Blob(u32),
    Object(u32),
}

#[derive(Debug, Clone)]
pub struct DrmModeBlob {
    data: Vec<u8>,
}

impl DrmModeBlob {
    pub fn new(data: Vec<u8>) -> Self {
        Self {
            data
        }
    }

    pub fn data(&self) -> Vec<u8> {
        self.data.clone()
    }

    pub fn length(&self) -> usize {
        self.data.len()
    }
}

impl DrmObjectCast for DrmModeBlob {
    fn cast(obj: &DrmObject) -> Option<&Arc<Self>> {
        if let DrmObject::Blob(b) = obj {
            Some(b)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone)]
pub struct DrmProperty {
    name: [u8; DRM_PROP_NAME_LEN],
    flags: PropertyFlags,
    kind: PropertyKind,
}

impl DrmProperty {
    pub fn name(&self) -> [u8; DRM_PROP_NAME_LEN] {
        self.name
    }

    pub fn flags(&self) -> u32 {
        self.flags.bits()
    }

    pub fn kind(&self) -> &PropertyKind {
        &self.kind
    }

    pub fn create(name: &str, flags: PropertyFlags) -> Self {
        let name = str_to_u8_32(name);
        Self {
            name,
            flags,
            kind: PropertyKind::Blob(0),
        }
    }

    pub fn create_bool(name: &str, flags: PropertyFlags) -> Self {
        Self::create_range(name, flags, 0, 1)
    }

    pub fn create_range(name: &str, flags: PropertyFlags, min: u64, max: u64) -> Self {
        Self {
            name: str_to_u8_32(name),
            flags: flags | PropertyFlags::RANGE,
            kind: PropertyKind::Range { min, max },
        }
    }

    pub fn create_enum(name: &str, flags: PropertyFlags, enums: Vec<PropertyEnum>) -> Self {
        Self {
            name: str_to_u8_32(name),
            flags: flags | PropertyFlags::ENUM,
            kind: PropertyKind::Enum(enums),
        }
    }

    pub fn count_values(&self) -> u32 {
        match &self.kind {
            PropertyKind::Range { .. } => 2,
            PropertyKind::SignedRange { .. } => 2,
            _ => 0
        }
    }

    pub fn count_enum_blobs(&self) -> u32 {
        match &self.kind {
            PropertyKind::Enum(entries) => entries.len() as u32,
            PropertyKind::Bitmask(entries) => entries.len() as u32,
            _ => 0,
        }
    }
}

impl DrmObjectCast for DrmProperty {
    fn cast(obj: &DrmObject) -> Option<&Arc<Self>> {
        if let DrmObject::Property(p) = obj {
            Some(p)
        } else {
            None
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct PropertyEnum {
    pub value: u64,
    pub name: [u8; DRM_PROP_NAME_LEN],
}

impl PropertyEnum {
    pub fn new(value: u64, name: &str) -> Self {
        Self {
            value,
            name: str_to_u8_32(name),
        }
    }
}

pub trait PropertySpec: Debug + Any {
    fn build(&self) -> DrmProperty;
}
