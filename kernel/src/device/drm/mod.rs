use aster_gpu::drm::{DrmDevice, DrmFeatures};

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

pub(super) fn init_in_first_kthread() -> Result<()> {
    let gpus = aster_gpu::registered_devices();

    if gpus.is_empty() {
        return_errno_with_message!(Errno::ENODEV, "no GPU devices registered");
    }

    let mut any_success = false;

    for (index, dev) in gpus.iter().enumerate() {
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
    if device.check_feature(DrmFeatures::COMPUTE_ACCEL) {
        let drm_minor = DrmMinor::new(index, device.clone(), DrmMinorType::Accel);
        char::register(drm_minor)?;
    } else {
        if device.check_feature(DrmFeatures::RENDER) {
            let drm_minor = DrmMinor::new(index, device.clone(), DrmMinorType::Render);
            char::register(drm_minor)?;
        }

        let drm_minor = DrmMinor::new(index, device.clone(), DrmMinorType::Primary);
        char::register(drm_minor)?;
    }

    Ok(())
}
