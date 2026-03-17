use alloc::vec;

use crate::drm::mode_object::{plane::DrmPlaneType, property::{DrmProperty, DrmPropertyFlags, PropertyEnum, PropertySpec}};

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
                ]
            ),
            Self::SrcX => todo!(),
            Self::SrcY => todo!(),
            Self::SrcW => todo!(),
            Self::SrcH => todo!(),
            Self::CrtcX => todo!(),
            Self::CrtcY => todo!(),
            Self::CrtcW => todo!(),
            Self::CrtcH => todo!(),
            Self::FbId => todo!(),
            Self::CrtcId => todo!(),
            Self::InFormats => todo!(),
        }
    }
}
