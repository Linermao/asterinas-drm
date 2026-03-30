use crate::drm::objects::property::{DrmProperty, DrmPropertyFlags, PropertySpec};

#[derive(Debug)]
pub enum CrtcProps {
    Active,
    ModeId,
}

impl PropertySpec for CrtcProps {
    fn build(&self) -> DrmProperty {
        match self {
            Self::Active => DrmProperty::create_bool("ACTIVE", DrmPropertyFlags::empty()),
            Self::ModeId => DrmProperty::create("MODE_ID", DrmPropertyFlags::BLOB),
        }
    }
}
