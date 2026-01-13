mod device;
mod driver;
mod file;

use hashbrown::HashMap;

use crate::{
    device::{
        drm::{
            device::{DrmDevice, DrmMinor, DrmMinorType},
            driver::{DrmDriver, DrmDriverFeatures, simple_drm},
        },
        registry::char,
    },
    prelude::*,
};

type DriverTable = HashMap<String, Arc<dyn DrmDriver>>;

fn build_driver_table() -> DriverTable {
    let mut table = DriverTable::new();

    // Register all available DRM drivers into the global driver table.
    // Each driver advertises its matching criteria and probe callbacks,
    // allowing the DRM core to select and bind a suitable driver when a
    // compatible GpuDevice is discovered.
    simple_drm::register_driver(&mut table);
    // virtio_drm::register_driver(&mut table);

    table
}

pub(super) fn init_in_first_kthread() -> Result<()> {
    simple_drm::init();

    let driver_table = build_driver_table();

    let gpus = aster_gpu::registered_devices();

    if gpus.is_empty() {
        return_errno_with_message!(Errno::ENODEV, "no GPU devices registered");
    }

    let mut any_success = false;

    // TODO: Do not rely on device.name() for driver matching.
    // 
    // Matching GpuDevice and DrmDriver, if matched, create DrmDevice.
    // Introduce a capability- or ID-based matching interface between GpuDevice and
    // DrmDriver to enable precise, extensible, and bus-agnostic driver selection.
    for (index, device) in gpus.iter().enumerate() {
        if let Some(driver) = driver_table.get(device.name()) {
            if driver.create_device(index as u32).is_ok() {
                any_success = true;
                // println!("[kernel] gpu device: {:?} probe correctly!", device.name());
            }
        }
    }

    if any_success {
        Ok(())
    } else {
        return_errno_with_message!(Errno::ENODEV, "all GPU devices register failed");
    }
}

fn drm_dev_register<D: DrmDriver>(device: Arc<DrmDevice<D>>) -> Result<()> {
	if device.check_feature(DrmDriverFeatures::COMPUTE_ACCEL) {
        let drm_minor = DrmMinor::new(device.clone(), DrmMinorType::Accel);
        char::register(drm_minor)?;
	} else {
        if device.check_feature(DrmDriverFeatures::RENDER) {
            let drm_minor = DrmMinor::new(device.clone(), DrmMinorType::Render);
            char::register(drm_minor)?;
        }

        let drm_minor = DrmMinor::new(device.clone(), DrmMinorType::Primary);
        char::register(drm_minor)?;
    }

    Ok(())
}
