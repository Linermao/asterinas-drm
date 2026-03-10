use alloc::vec;

use crate::drm::mode_object::property::{DrmProperty, PropertyEnum, PropertyFlags, PropertySpec};

#[repr(u64)]
#[derive(Debug)]
enum DrmModeDpms {
    ON = 0,
    STANDBY = 1,
    SUSPEND = 2,
    OFF = 3,
}

#[repr(u64)]
#[derive(Debug)]
enum DrmLinkStatus {
    GOOD = 0,
    BAD = 1,
}

#[derive(Debug)]
pub enum ConnectorProps {
    DPMS,
    // PATH(u32),
    TILE,
    LinkStatus,
    NonDesktop,
    // HdrOutputMetadata,
}

impl PropertySpec for ConnectorProps {
    fn build(&self) -> DrmProperty {
        match self {
            Self::DPMS => DrmProperty::create_enum(
                "DPMS",
                PropertyFlags::empty(),
                vec![
                    PropertyEnum::new(DrmModeDpms::ON as u64, "On"),
                    PropertyEnum::new(DrmModeDpms::STANDBY as u64, "Standby"),
                    PropertyEnum::new(DrmModeDpms::SUSPEND as u64, "Suspend"),
                    PropertyEnum::new(DrmModeDpms::OFF as u64, "Off"),
                ],
            ),
            Self::LinkStatus => DrmProperty::create_enum(
                "LinkStatus",
                PropertyFlags::empty(),
                vec![
                    PropertyEnum::new(DrmLinkStatus::GOOD as u64, "Good"),
                    PropertyEnum::new(DrmLinkStatus::BAD as u64, "Bad"),
                ],
            ),
            Self::NonDesktop => DrmProperty::create_bool("NonDesktop", PropertyFlags::IMMUTABLE),
            Self::TILE => {
                DrmProperty::create("Tile", PropertyFlags::BLOB | PropertyFlags::IMMUTABLE)
            }
        }
    }
}
