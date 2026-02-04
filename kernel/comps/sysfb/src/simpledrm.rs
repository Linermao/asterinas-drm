use alloc::{boxed::Box, sync::Arc};

use aster_gpu::{
    GpuDevice,
    drm::{
        DrmError,
        device::DrmDevice,
        driver::{DrmDriver, DrmDriverFeatures, DrmDriverOps, DumbCreateProvider},
        gem::DrmGemObject,
        mode_config::{
            DrmModeConfig, DrmModeModeInfo,
            connector::{ConnectorStatus, DrmConnector, funcs::ConnectorFuncs},
            crtc::{DrmCrtc, funcs::CrtcFuncs},
            encoder::{DrmEncoder, EncoderType, funcs::EncoderFuncs},
            framebuffer::{DrmFramebuffer, funcs::FramebufferFuncs},
            funcs::ModeConfigFuncs,
            plane::{DrmPlane, PlaneType, funcs::PlaneFuncs},
        },
    },
};

const SIMPLEDRM_NAME: &'static str = "simpledrm";
const SIMPLEDRM_DESC: &'static str = "DRM driver for simple-framebuffer platform devices";
const SIMPLEDRM_DATE: &'static str = "2025-01-02";

#[derive(Debug)]
pub struct SimpleDrmDevice {
    device: Arc<DrmDevice>,
}

impl SimpleDrmDevice {
    fn new(index: u32) -> Result<Self, DrmError> {
        // TODO: get the hardware format to set this properties
        let min_width = 1;
        let max_width = 8192;
        let min_height = 1;
        let max_height = 8192;
        let preferred_depth = 16;

        let mut mode_config = DrmModeConfig::new(
            min_width,
            max_width,
            min_height,
            max_height,
            preferred_depth,
            Box::new(SimpleModeConfigFuncs {}),
        );

        // Drm Objects initial
        let primary_plane = DrmPlane::init(
            &mut mode_config,
            PlaneType::Primary,
            Box::new(SimplePlaneFuncs),
        )?;
        let crtc = DrmCrtc::init_with_planes(
            &mut mode_config,
            None,
            primary_plane,
            None,
            Box::new(SimpleCrtcFuncs),
        )?;
        let encoder = DrmEncoder::init_with_crtcs(
            &mut mode_config,
            EncoderType::VIRTUAL,
            &[crtc],
            Box::new(SimpleEncoderFuncs),
        )?;

        let fake_modeinfo = fake_modeinfo();
        let _connector = DrmConnector::init_with_encoder(
            &mut mode_config,
            ConnectorStatus::Connected,
            &[fake_modeinfo],
            &[encoder],
            Box::new(SimpleConnectorFuncs),
        )?;

        mode_config.init_standard_properties();

        let driver = Arc::new(SimpleDrmDriver {});
        // TODO: initialize device-specific features
        // In Linux, a drm_device's features are not necessarily identical to
        // its driver's features. Here we start from the driver-wide features
        // and adjust per-device settings (e.g., enable/disable render node
        // or other capabilities for this specific device instance).
        let driver_features = driver.driver_features();
        let device = Arc::new(DrmDevice::new(index, driver, driver_features, mode_config));

        Ok(Self { device })
    }
}

#[derive(Debug)]
struct SimpleDrmDriver {}

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

    fn create_device(
        &self,
        index: u32,
        _gpu_device: Arc<dyn GpuDevice>,
    ) -> Result<Arc<DrmDevice>, DrmError> {
        let sdev = SimpleDrmDevice::new(index)?;
        Ok(sdev.device.clone())
    }

    fn driver_features(&self) -> DrmDriverFeatures {
        DrmDriverFeatures::GEM | DrmDriverFeatures::MODESET
    }

    fn driver_ops(&self) -> DrmDriverOps {
        DrmDriverOps {
            dumb_create: Some(DumbCreateProvider::Memfd),
        }
    }
}

#[derive(Debug)]
struct SimpleModeConfigFuncs;

impl ModeConfigFuncs for SimpleModeConfigFuncs {
    fn create_framebuffer(
        &self,
        width: u32,
        height: u32,
        pitch: u32,
        bpp: u32,
        gem_obj: Arc<DrmGemObject>,
    ) -> Result<DrmFramebuffer, DrmError> {
        Ok(DrmFramebuffer::new(
            width,
            height,
            pitch,
            bpp,
            gem_obj,
            Box::new(SimpleFramebufferFuncs {}),
        ))
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

#[derive(Debug)]
struct SimpleFramebufferFuncs;

impl PlaneFuncs for SimplePlaneFuncs {}

impl CrtcFuncs for SimpleCrtcFuncs {}

impl EncoderFuncs for SimpleEncoderFuncs {}

impl ConnectorFuncs for SimpleConnectorFuncs {}

impl FramebufferFuncs for SimpleFramebufferFuncs {}

#[derive(Debug)]
struct SimpleGpuDevice;

impl GpuDevice for SimpleGpuDevice {
    fn driver_name(&self) -> &str {
        SIMPLEDRM_NAME
    }
}

pub fn register_device() {
    let device = Arc::new(SimpleGpuDevice {});
    aster_gpu::register_device(device).expect("failed to register simple_drm GpuDevice");
}

pub fn register_driver() {
    let driver = Arc::new(SimpleDrmDriver {});
    aster_gpu::register_driver(SIMPLEDRM_NAME, driver)
        .expect("failed to register simple_drm DrmDriver");
}

// Create a fake display mode for testing and bring-up purposes.
//
// This mode is not obtained from real hardware (e.g. EDID or firmware).
// It provides a minimal, hard-coded timing description that allows the
// DRM pipeline to be exercised during early development, testing, or
// virtualized environments (such as simpledrm, QEMU, or headless setups).
//
// The values are chosen to represent a common 1280x800@60Hz mode and are
// sufficient for validating mode-setting, atomic state transitions, and
// userspace interaction. Real drivers must replace this with modes derived
// from hardware capabilities or display discovery mechanisms.
fn fake_modeinfo() -> DrmModeModeInfo {
    let mut name = [0u8; 32];
    let bytes = "1280x800".as_bytes();
    let len = bytes.len().min(32);
    name[..len].copy_from_slice(&bytes[..len]);

    DrmModeModeInfo {
        clock: 65000, // kHz (65 MHz)

        hdisplay: 1280,
        hsync_start: 1048,
        hsync_end: 1184,
        htotal: 1344,

        hskew: 0,

        vdisplay: 800,
        vsync_start: 771,
        vsync_end: 777,
        vtotal: 806,

        vscan: 0,

        vrefresh: 60,

        flags: 0x5,  // DRM_MODE_FLAG_PHSYNC | DRM_MODE_FLAG_PVSYNC
        type_: 0x40, // DRM_MODE_TYPE_DRIVER (0x40) or DRIVER | PREFERRED (0x60)

        name,
    }
}
