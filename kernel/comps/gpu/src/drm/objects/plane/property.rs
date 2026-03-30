use alloc::vec;

use crate::drm::objects::{
    plane::DrmPlaneType,
    property::{DrmProperty, DrmPropertyFlags, PropertyEnum, PropertySpec},
};

#[derive(Debug)]
pub enum PlaneProps {
    Type,
    SrcX,
    SrcY,
    SrcW,
    SrcH,
    CrtcX,
    CrtcY,
    CrtcW,
    CrtcH,
    FbId,
    CrtcId,
    InFormats,
}

impl PropertySpec for PlaneProps {
    fn build(&self) -> DrmProperty {
        match self {
            Self::Type => DrmProperty::create_enum(
                "type",
                DrmPropertyFlags::empty(),
                vec![
                    PropertyEnum::new(DrmPlaneType::Primary as u64, "Primary"),
                    PropertyEnum::new(DrmPlaneType::Overlay as u64, "Overlay"),
                    PropertyEnum::new(DrmPlaneType::Cursor as u64, "Cursor"),
                ],
            ),
            Self::SrcX => {
                DrmProperty::create_range("SRC_X", DrmPropertyFlags::empty(), 0, u32::MAX as u64)
            }
            Self::SrcY => {
                DrmProperty::create_range("SRC_Y", DrmPropertyFlags::empty(), 0, u32::MAX as u64)
            }
            Self::SrcW => {
                DrmProperty::create_range("SRC_W", DrmPropertyFlags::empty(), 0, u32::MAX as u64)
            }
            Self::SrcH => {
                DrmProperty::create_range("SRC_H", DrmPropertyFlags::empty(), 0, u32::MAX as u64)
            }
            Self::CrtcX => {
                DrmProperty::create_range("CRTC_X", DrmPropertyFlags::empty(), 0, u32::MAX as u64)
            }
            Self::CrtcY => {
                DrmProperty::create_range("CRTC_Y", DrmPropertyFlags::empty(), 0, u32::MAX as u64)
            }
            Self::CrtcW => {
                DrmProperty::create_range("CRTC_W", DrmPropertyFlags::empty(), 0, u32::MAX as u64)
            }
            Self::CrtcH => {
                DrmProperty::create_range("CRTC_H", DrmPropertyFlags::empty(), 0, u32::MAX as u64)
            }
            Self::FbId => DrmProperty::create("FB_ID", DrmPropertyFlags::empty()),
            Self::CrtcId => DrmProperty::create("CRTC_ID", DrmPropertyFlags::empty()),
            Self::InFormats => DrmProperty::create(
                "IN_FORMATS",
                DrmPropertyFlags::BLOB | DrmPropertyFlags::IMMUTABLE,
            ),
        }
    }
}
