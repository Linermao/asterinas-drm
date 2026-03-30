use alloc::{sync::Arc, vec::Vec};
use core::{any::Any, fmt::Debug};

use hashbrown::HashMap;

use crate::drm::objects::{DrmObject, DrmObjectCast};

pub const DRM_PROP_NAME_LEN: usize = 32;
pub type PropertyObject = HashMap<u32, u64>;

fn str_to_u8(s: &str) -> [u8; DRM_PROP_NAME_LEN] {
    let mut buf = [0u8; DRM_PROP_NAME_LEN];

    let bytes = s.as_bytes();
    let len = bytes.len().min(DRM_PROP_NAME_LEN - 1);

    buf[..len].copy_from_slice(&bytes[..len]);
    buf
}

bitflags::bitflags! {
    pub struct DrmPropertyFlags: u32 {
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
pub enum DrmObjectType {
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
pub enum DrmPropertyKind {
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
        Self { data }
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
    name: &'static str,
    flags: DrmPropertyFlags,
    kind: DrmPropertyKind,
    type_: DrmPropertyType,
}

impl DrmProperty {
    pub fn name(&self) -> &'static str {
        self.name
    }

    pub fn name_to_u8(&self) -> [u8; DRM_PROP_NAME_LEN] {
        str_to_u8(self.name)
    }

    pub fn type_(&self) -> DrmPropertyType {
        self.type_
    }

    pub fn flags(&self) -> u32 {
        self.flags.bits()
    }

    pub fn kind(&self) -> &DrmPropertyKind {
        &self.kind
    }

    pub fn create(name: &'static str, flags: DrmPropertyFlags) -> Self {
        Self {
            name,
            flags,
            kind: DrmPropertyKind::Blob(0),
            type_: DrmPropertyType::from_name(name)
        }
    }

    pub fn create_bool(name: &'static str, flags: DrmPropertyFlags) -> Self {
        Self::create_range(name, flags, 0, 1)
    }

    pub fn create_range(name: &'static str, flags: DrmPropertyFlags, min: u64, max: u64) -> Self {
        Self {
            name,
            flags: flags | DrmPropertyFlags::RANGE,
            kind: DrmPropertyKind::Range { min, max },
            type_: DrmPropertyType::from_name(name)
        }
    }

    pub fn create_enum(name: &'static str, flags: DrmPropertyFlags, enums: Vec<PropertyEnum>) -> Self {
        Self {
            name,
            flags: flags | DrmPropertyFlags::ENUM,
            kind: DrmPropertyKind::Enum(enums),
            type_: DrmPropertyType::from_name(name)
        }
    }

    pub fn count_values(&self) -> u32 {
        match &self.kind {
            DrmPropertyKind::Range { .. } => 2,
            DrmPropertyKind::SignedRange { .. } => 2,
            _ => 0,
        }
    }

    pub fn count_enum_blobs(&self) -> u32 {
        match &self.kind {
            DrmPropertyKind::Enum(entries) => entries.len() as u32,
            DrmPropertyKind::Bitmask(entries) => entries.len() as u32,
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

#[derive(Debug, Clone, Copy)]
pub enum DrmPropertyType {
    CrtcId,
    FbId,
    Active,
    ModeId,
    OutFencePtr,
    InFenceFd,
    SrcX,
    SrcY,
    SrcW,
    SrcH,
    CrtcX,
    CrtcY,
    CrtcW,
    CrtcH,
    Unknown,
}

impl DrmPropertyType {
    pub fn from_name(name: &str) -> Self {
        match name {
            "CRTC_ID" => Self::CrtcId,
            "FB_ID" => Self::FbId,
            "ACTIVE" => Self::Active,
            "MODE_ID" => Self::ModeId,
            "OUT_FENCE_PTR" => Self::OutFencePtr,
            "IN_FENCE_FD" => Self::InFenceFd,
            "SRC_X" => Self::SrcX,
            "SRC_Y" => Self::SrcY,
            "SRC_W" => Self::SrcW,
            "SRC_H" => Self::SrcH,
            "CRTC_X" => Self::CrtcX,
            "CRTC_Y" => Self::CrtcY,
            "CRTC_W" => Self::CrtcW,
            "CRTC_H" => Self::CrtcH,
            _ => Self::Unknown,
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
    pub fn new(value: u64, name: &'static str) -> Self {
        Self {
            value,
            name: str_to_u8(name),
        }
    }
}

pub trait PropertySpec: Debug + Any {
    fn build(&self) -> DrmProperty;
}
