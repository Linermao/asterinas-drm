use alloc::vec;

use crate::drm::objects::property::{DrmProperty, DrmPropertyFlags, PropertyEnum, PropertySpec};

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
    Tile,
    LinkStatus,
    NonDesktop,
    // HdrOutputMetadata,
    CrtcId,
}

impl PropertySpec for ConnectorProps {
    fn build(&self) -> DrmProperty {
        match self {
            Self::DPMS => DrmProperty::create_enum(
                "DPMS",
                DrmPropertyFlags::empty(),
                vec![
                    PropertyEnum::new(DrmModeDpms::ON as u64, "On"),
                    PropertyEnum::new(DrmModeDpms::STANDBY as u64, "Standby"),
                    PropertyEnum::new(DrmModeDpms::SUSPEND as u64, "Suspend"),
                    PropertyEnum::new(DrmModeDpms::OFF as u64, "Off"),
                ],
            ),
            Self::LinkStatus => DrmProperty::create_enum(
                "LinkStatus",
                DrmPropertyFlags::empty(),
                vec![
                    PropertyEnum::new(DrmLinkStatus::GOOD as u64, "Good"),
                    PropertyEnum::new(DrmLinkStatus::BAD as u64, "Bad"),
                ],
            ),
            Self::NonDesktop => DrmProperty::create_bool("NonDesktop", DrmPropertyFlags::IMMUTABLE),
            Self::Tile => {
                DrmProperty::create("Tile", DrmPropertyFlags::BLOB | DrmPropertyFlags::IMMUTABLE)
            }
            Self::CrtcId => DrmProperty::create("CRTC_ID", DrmPropertyFlags::empty()),
        }
    }
}
