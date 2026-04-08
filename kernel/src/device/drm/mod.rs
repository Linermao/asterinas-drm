use core::sync::atomic::{AtomicU64, Ordering};

use aster_gpu::drm::{DrmDevice, DrmFeatures};
use hashbrown::HashMap;

use crate::{
    device::{
        drm::minor::{DrmMinor, DrmMinorType},
        registry::char,
    },
    prelude::*,
};

mod file;
mod ioctl_defs;
mod memfd;
mod minor;
mod syncobj;

#[derive(Debug)]
struct DrmVmaOffsetManager {
    base: u64,
    next_offset: AtomicU64,
    offset_to_handle: HashMap<u64, u32>,
}

impl DrmVmaOffsetManager {
    pub fn new() -> Self {
        Self {
            base: 0x10_0000,
            next_offset: AtomicU64::new(0),
            offset_to_handle: HashMap::new(),
        }
    }

    pub fn alloc(&mut self, handle: u32) -> Result<u64> {
        let offset = self.base
            + self
                .next_offset
                .fetch_add(PAGE_SIZE as u64, Ordering::SeqCst);
        self.offset_to_handle.insert(offset, handle);
        Ok(offset)
    }

    pub fn lookup(&self, offset: u64) -> Option<u32> {
        self.offset_to_handle.get(&offset).copied()
    }

    pub fn free(&mut self, offset: u64) {
        self.offset_to_handle.remove(&offset);
    }
}

#[derive(Debug)]
struct DrmDeviceCore {
    device: Arc<dyn DrmDevice>,
    vma_manager: Mutex<DrmVmaOffsetManager>,
} 

pub(super) fn init_in_first_kthread() -> Result<()> {
    let gpus = aster_gpu::registered_devices();

    if gpus.is_empty() {
        return_errno_with_message!(Errno::ENODEV, "no GPU devices registered");
    }

    let mut any_success = false;

    for (index, dev) in gpus.iter().enumerate() {
        // TODO:
        if dev.name() == "simpledrm" && gpus.len() > 1 {
            continue;
        }
        
        match drm_dev_register(index as u32, dev) {
            Ok(_) => {
                // log::info!("[kernel] gpu driver {:?} probe correctly!", device.name());
                any_success = true;
            }
            Err(error) => {
                log::error!("[kernel] DrmDevice create error: {:?}", error);
            }
        }
    }

    if any_success {
        Ok(())
    } else {
        return_errno_with_message!(Errno::ENODEV, "all GPU devices register failed");
    }
}

fn drm_dev_register(index: u32, device: &Arc<dyn DrmDevice>) -> Result<()> {

    let device_core = Arc::new(DrmDeviceCore {
        device: device.clone(),
        vma_manager: Mutex::new(DrmVmaOffsetManager::new()),
    });

    if device.contain_features(DrmFeatures::COMPUTE_ACCEL) {
        let drm_minor = DrmMinor::new(index, device_core.clone(), DrmMinorType::Accel);
        char::register(drm_minor)?;
    } else {
        if device.contain_features(DrmFeatures::RENDER) {
            let drm_minor = DrmMinor::new(index, device_core.clone(), DrmMinorType::Render);
            char::register(drm_minor)?;
        }

        let drm_minor = DrmMinor::new(index, device_core.clone(), DrmMinorType::Primary);
        char::register(drm_minor)?;
    }

    Ok(())
}
