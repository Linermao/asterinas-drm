use alloc::sync::Arc;

use aster_gpu::GpuDevice;

use crate::{
    device::drm::{
        DrmDevice, DrmDriver, DrmMinorType, driver::DrmDriverFeatures, drm_dev_register,
    },
    drm_register_driver,
    prelude::*,
};

const SIMPLEDRM_NAME: &'static str = "simpledrm";
const SIMPLEDRM_DESC: &'static str = "DRM driver for simple-framebuffer platform devices";
const SIMPLEDRM_DATE: &'static str = "2025-01-02";

#[derive(Debug)]
struct SimpleDrmDevice {
    device: Arc<DrmDevice<SimpleDrmDriver>>,
}

impl SimpleDrmDevice {
    fn new(index: u32) -> Self {
        let driver = Arc::new(SimpleDrmDriver {});
        // TODO: initialize device-specific features
        // In Linux, a drm_device's features are not necessarily identical to
        // its driver's features. Here we start from the driver-wide features
        // and adjust per-device settings (e.g., enable/disable render node
        // or other capabilities for this specific device instance).
        let driver_features = driver.driver_features();
        let device = Arc::new(DrmDevice::new(index, driver, driver_features));

        Self { device }
    }

    fn init(&self) -> Result<()> {
        // TODO: Register the DRM object here, including setting up
        // its properties and associating the relevant object functions.

        Ok(())
    }
}

drm_register_driver!(SimpleDrmDriver, SIMPLEDRM_NAME);

impl DrmDriver for SimpleDrmDriver {
    fn name(&self) -> &str {
        SIMPLEDRM_NAME
    }

    fn desc(&self) -> &str {
        SIMPLEDRM_DESC
    }

    fn date(&self) -> &str {
        SIMPLEDRM_DATE
    }

    fn create_device(&self, index: u32) -> Result<()> {
        let sdev = SimpleDrmDevice::new(index);
        sdev.init()?;

        // register drm device
        drm_dev_register(sdev.device.clone())?;

        Ok(())
    }

    fn driver_features(&self) -> DrmDriverFeatures {
        DrmDriverFeatures::ATOMIC 
        | DrmDriverFeatures::GEM
        | DrmDriverFeatures::MODESET 
    }
}

#[derive(Debug)]
struct SimpleGpuDevice;

impl GpuDevice for SimpleGpuDevice {
    fn name(&self) -> &str {
        SIMPLEDRM_NAME
    }
}

pub fn init() {
    let device = Arc::new(SimpleGpuDevice {});
    aster_gpu::register_device(device).expect("failed to register simple_drm GpuDevice");
}
