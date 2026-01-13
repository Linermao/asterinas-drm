use alloc::{format, sync::Arc};

use device_id::{DeviceId, MajorId, MinorId};

use crate::{
    device::drm::{DrmDriver, driver::DrmDriverFeatures, file::DrmFile, mode_config::DrmModeConfig},
    fs::{
        device::{Device, DeviceType},
        inode_handle::FileIo,
    },
    prelude::*,
};

const DRM_MAJOR_ID: u16 = 226;
const RENDER_MINOR_BASE: u32 = 128;

/// Represents a DRM device instance bound to a specific DRM driver.
///
/// `DrmDevice` models the core DRM object that owns global device state and
/// driver association. It is created during driver probing and serves as the
/// shared backend for one or more `DrmMinor` nodes.
///
/// A single `DrmDevice` may give rise to multiple minors (primary, render,
/// control), all of which reference this structure and share the same driver
/// instance. Per-minor differences (such as permissions and ioctl exposure)
/// are handled at the `DrmMinor` and file level, not here.
///
/// This structure is not directly exposed to userspace; it exists to:
/// - Bind a DRM driver to a concrete device instance
/// - Act as the common anchor point for all associated minors
#[derive(Debug)]
pub(super) struct DrmDevice<D: DrmDriver> {
    index: u32,

    driver: Arc<D>,
    /// Feature flags and capability bits advertised by the driver for this
    /// device instance.
    ///
    /// These flags describe supported DRM functionality and are used by
    /// userspace (via ioctls) and the DRM core to gate behavior.
    driver_features: DrmDriverFeatures,

    mode_config: Mutex<DrmModeConfig>,
}

impl<D: DrmDriver> DrmDevice<D> {
    pub fn new(index: u32, driver: Arc<D>, driver_features: DrmDriverFeatures) -> Self {
        Self {
            index,
            driver,
            driver_features,
            mode_config: Mutex::new(DrmModeConfig::default()),
        }
    }

    pub fn resources(&self) -> &Mutex<DrmModeConfig> {
        &self.mode_config
    }

    pub fn check_feature(&self, features: DrmDriverFeatures) -> bool {
        self.driver_features.contains(features)
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) enum DrmMinorType {
    Primary = 0,
    Control = 1,
    Render = 2,
    Accel = 32,
}

/// Represents a DRM minor node exposed to userspace (e.g. primary, render,
/// or control node).
///
/// A `DrmMinor` corresponds to a single character device registered under
/// `/dev/dri/` (such as `/dev/dri/cardX` or `/dev/dri/renderDX`). It does not
/// own hardware state by itself; instead, it provides a userspace-facing
/// access point with a specific permission and usage model.
///
/// Multiple `DrmMinor` instances may reference the same underlying
/// `DrmDevice`, sharing the same driver instance and global device state.
/// The semantic differences between minors (e.g. authentication requirements,
/// ioctl visibility, access restrictions) are expressed via `type_` and
/// enforced at the file/ioctl level.
#[derive(Debug)]
pub(super) struct DrmMinor<D: DrmDriver> {
    /// The same index as DrmDevice<D>
    index: u32,
    /// The type of this minor node (primary, render, control, etc).
    ///
    /// This determines permission checks, supported ioctl sets,
    /// and access semantics enforced by the DRM core.
    type_: DrmMinorType,

    device: Arc<DrmDevice<D>>,

    weak_self: Weak<Self>,
}

impl<D: DrmDriver> DrmMinor<D> {
    pub fn new(device: Arc<DrmDevice<D>>, type_: DrmMinorType) -> Arc<Self> {
        Arc::new_cyclic(move |weak_ref| {
            Self {
                index: device.index,
                type_,
                device,
                weak_self: weak_ref.clone(),
            }
        })
    }
        
    pub fn resources(&self) -> &Mutex<DrmModeConfig> {
        &self.device.resources()
    }

    pub fn check_feature(&self, features: DrmDriverFeatures) -> bool {
        self.device.check_feature(features)
    }
}

impl<D: DrmDriver> Device for DrmMinor<D> {
    fn type_(&self) -> DeviceType {
        DeviceType::Char
    }

    fn id(&self) -> DeviceId {
        let mut minor_id = self.index;
        match self.type_ {
            DrmMinorType::Render => {
                minor_id += RENDER_MINOR_BASE;
            }
            _ => {}
        }
        DeviceId::new(MajorId::new(DRM_MAJOR_ID), MinorId::new(minor_id))
    }

    fn devtmpfs_path(&self) -> Option<String> {
        match self.type_ {
            DrmMinorType::Primary => Some(format!("dri/card{}", self.index)),
            DrmMinorType::Render => Some(format!("dri/render{}", RENDER_MINOR_BASE + self.index)),
            DrmMinorType::Control => Some(format!("dri/controlD{}", self.index)),
            _ => None,
        }
    }

    fn open(&self) -> Result<Box<dyn FileIo>> {
        Ok(Box::new(DrmFile::new(self.weak_self.upgrade().unwrap())))
    }
}
