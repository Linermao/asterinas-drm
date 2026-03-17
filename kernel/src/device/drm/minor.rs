use alloc::{format, sync::Arc};

use aster_gpu::drm::DrmDevice;
use device_id::{DeviceId, MajorId, MinorId};

use crate::{
    device::drm::file::DrmFile,
    fs::{
        device::{Device, DeviceType},
        inode_handle::FileIo,
    },
    prelude::*,
};

const DRM_MAJOR_ID: u16 = 226;
const CONTROL_MINOR_BASE: u32 = 64;
const RENDER_MINOR_BASE: u32 = 128;

#[derive(Debug)]
pub(super) enum DrmMinorType {
    Primary = 0,
    #[expect(dead_code)]
    Control = 1,
    Render = 2,
    Accel = 32,
}

#[derive(Debug)]
pub(super) struct DrmMinor {
    index: u32,
    type_: DrmMinorType,

    device: Arc<dyn DrmDevice>,

    weak_self: Weak<Self>,
}

impl DrmMinor {
    pub fn new(index: u32, device: Arc<dyn DrmDevice>, type_: DrmMinorType) -> Arc<Self> {
        Arc::new_cyclic(move |weak_ref| Self {
            index,
            type_,
            device,
            weak_self: weak_ref.clone(),
        })
    }

    pub fn device(&self) -> Arc<dyn DrmDevice> {
        self.device.clone()
    }
}

impl Device for DrmMinor {
    fn type_(&self) -> DeviceType {
        DeviceType::Char
    }

    fn id(&self) -> DeviceId {
        let mut minor_id = self.index;
        match self.type_ {
            DrmMinorType::Render => {
                minor_id += RENDER_MINOR_BASE;
            }
            DrmMinorType::Control => {
                minor_id += CONTROL_MINOR_BASE;
            }
            _ => {}
        }
        DeviceId::new(MajorId::new(DRM_MAJOR_ID), MinorId::new(minor_id))
    }

    fn devtmpfs_path(&self) -> Option<String> {
        match self.type_ {
            DrmMinorType::Primary => Some(format!("dri/card{}", self.index)),
            DrmMinorType::Render => Some(format!("dri/renderD{}", RENDER_MINOR_BASE + self.index)),
            DrmMinorType::Control => {
                Some(format!("dri/controlD{}", CONTROL_MINOR_BASE + self.index))
            }
            _ => None,
        }
    }

    fn open(&self) -> Result<Box<dyn FileIo>> {
        Ok(Box::new(DrmFile::new(self.weak_self.upgrade().unwrap())))
    }
}
