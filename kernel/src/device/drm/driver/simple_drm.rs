use alloc::sync::Arc;

use aster_gpu::GpuDevice;

use crate::{
    device::drm::{
        DrmDevice, DrmDriver,
        driver::{DrmDriverFeatures, fake_modeinfo},
        drm_dev_register,
        mode_config::{
            connector::{ConnectorStatus, DrmConnector, funcs::ConnectorFuncs},
            crtc::{DrmCrtc, funcs::CrtcFuncs},
            encoder::{DrmEncoder, EncoderType, funcs::EncoderFuncs},
            plane::{DrmPlane, PlaneType, funcs::PlaneFuncs},
        },
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
        let mut resources = self.device.resources().lock();
        resources.init_standard_properties();

        let primary_plane = DrmPlane::init(
            &mut resources,
            PlaneType::Primary,
            Box::new(SimplePlaneFuncs),
        )?;
        let crtc = DrmCrtc::init_with_planes(
            &mut resources,
            None,
            primary_plane,
            None,
            Box::new(SimpleCrtcFuncs),
        )?;
        let encoder = DrmEncoder::init_with_crtcs(
            &mut resources,
            EncoderType::VIRTUAL,
            &[crtc],
            Box::new(SimpleEncoderFuncs),
        )?;

        let fake_modeinfo = fake_modeinfo();
        let _connector = DrmConnector::init_with_encoder(
            &mut resources,
            ConnectorStatus::Connected,
            &[fake_modeinfo],
            &[encoder],
            Box::new(SimpleConnectorFuncs),
        )?;

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
        DrmDriverFeatures::ATOMIC | DrmDriverFeatures::GEM | DrmDriverFeatures::MODESET
    }
}

#[derive(Debug)]
struct SimplePlaneFuncs;

#[derive(Debug)]
struct SimpleCrtcFuncs;

#[derive(Debug)]
struct SimpleEncoderFuncs;

#[derive(Debug)]
struct SimpleConnectorFuncs;

impl PlaneFuncs for SimplePlaneFuncs {}

impl CrtcFuncs for SimpleCrtcFuncs {}

impl EncoderFuncs for SimpleEncoderFuncs {}

impl ConnectorFuncs for SimpleConnectorFuncs {}

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
