use alloc::{boxed::Box, format, sync::Arc};

use aster_gpu::drm::{
    DrmError,
    device::DrmDevice,
    ioctl::DrmModeCrtc,
    mode_config::{
        DrmModeConfig, DrmModeModeInfo,
        connector::{
            ConnectorStatus, DrmConnector,
            funcs::{ConnectorFuncs, drm_helper_probe_single_connector_modes},
        },
        crtc::{DrmCrtc, funcs::CrtcFuncs, helper::drm_atomic_helper_page_flip},
        encoder::{DrmEncoder, EncoderType, funcs::EncoderFuncs},
        framebuffer::DrmFramebuffer,
        plane::{DrmPlane, PlaneType, funcs::PlaneFuncs},
    },
    vblank::DrmPendingVblankEvent,
};

use crate::device::gpu::device::VirtioGpuDevice;

pub fn virtio_gpu_output_init(
    scanout: u32,
    mode_config: &mut DrmModeConfig,
    vgpu: &VirtioGpuDevice,
) -> Result<(), DrmError> {
    let primary = DrmPlane::init(mode_config, PlaneType::Primary, Box::new(VirtioPlaneFuncs))?;
    let cursor = DrmPlane::init(mode_config, PlaneType::Cursor, Box::new(VirtioPlaneFuncs))?;

    let crtc = DrmCrtc::init_with_planes(
        mode_config,
        None,
        primary,
        Some(cursor),
        Box::new(VirtioCrtcFuncs),
    )?;

    let encoder = DrmEncoder::init_with_crtcs(
        mode_config,
        EncoderType::VIRTUAL,
        &[crtc],
        Box::new(VirtioEncoderFuncs),
    )?;

    let connector =
        DrmConnector::init_with_encoder(mode_config, &[encoder], Box::new(VirtioConnectorFuncs))?;

    // TODO: connector.funcs.get_modes()
    let infos = vgpu.display_infos();
    if let Some(info) = infos.get(scanout as usize) {
        if info.enabled != 0 && info.rect.width != 0 && info.rect.height != 0 {
            let mode = mode_from_size(info.rect.width, info.rect.height);
            connector.update_modes(&[mode])?;
        }
    }

    // TODO: if (vgdev->has_edid)

    Ok(())
}

#[derive(Debug)]
struct VirtioPlaneFuncs;

#[derive(Debug)]
struct VirtioCrtcFuncs;

#[derive(Debug)]
struct VirtioEncoderFuncs;

#[derive(Debug)]
struct VirtioConnectorFuncs;

impl PlaneFuncs for VirtioPlaneFuncs {}

impl CrtcFuncs for VirtioCrtcFuncs {
    fn page_flip(
        &self,
        device: Arc<DrmDevice>,
        crtc: Arc<DrmCrtc>,
        fb: Arc<DrmFramebuffer>,
        event: Option<DrmPendingVblankEvent>,
        flags: u32,
        target: Option<u32>,
    ) -> Result<(), DrmError> {
        drm_atomic_helper_page_flip(device, crtc, fb, event, flags, target)
    }

    fn set_config(
        &self,
        crtc: Arc<DrmCrtc>,
        fb: Arc<DrmFramebuffer>,
        crtc_req: &DrmModeCrtc,
    ) -> Result<(), DrmError> {
        todo!()
    }

    fn enable_vblank(&self, crtc: Arc<DrmCrtc>) -> Result<(), DrmError> {
        todo!()
    }

    fn disable_vblank(&self, crtc: Arc<DrmCrtc>) -> Result<(), DrmError> {
        todo!()
    }
}

impl EncoderFuncs for VirtioEncoderFuncs {}

impl ConnectorFuncs for VirtioConnectorFuncs {
    fn fill_modes(
        &self,
        max_x: u32,
        max_y: u32,
        connector: Arc<DrmConnector>,
    ) -> Result<(), DrmError> {
        drm_helper_probe_single_connector_modes(max_x, max_y, connector)
    }

    fn detect(&self, _force: bool, connector: Arc<DrmConnector>) -> Result<(), DrmError> {
        connector.update_status(ConnectorStatus::Connected)
    }

    fn get_modes(&self, connector: Arc<DrmConnector>) -> Result<(), DrmError> {
        // TODO
        Ok(())
    }
}

// TODO: dirty
fn mode_from_size(width: u32, height: u32) -> DrmModeModeInfo {
    let hdisplay = width.min(u16::MAX as u32) as u16;
    let vdisplay = height.min(u16::MAX as u32) as u16;

    let hsync_start = hdisplay.saturating_add(48);
    let hsync_end = hdisplay.saturating_add(80);
    let htotal = hdisplay.saturating_add(160);

    let vsync_start = vdisplay.saturating_add(3);
    let vsync_end = vdisplay.saturating_add(6);
    let vtotal = vdisplay.saturating_add(28);

    let vrefresh: u32 = 60;
    let clock = (htotal as u32) * (vtotal as u32) * vrefresh / 1000;

    let mut name = [0u8; 32];
    let s = format!("{width}x{height}");
    let bytes = s.as_bytes();
    let len = bytes.len().min(32);
    name[..len].copy_from_slice(&bytes[..len]);

    DrmModeModeInfo {
        clock,

        hdisplay,
        hsync_start,
        hsync_end,
        htotal,
        hskew: 0,

        vdisplay,
        vsync_start,
        vsync_end,
        vtotal,
        vscan: 0,

        vrefresh,

        flags: 0x5,  // DRM_MODE_FLAG_PHSYNC | DRM_MODE_FLAG_PVSYNC
        type_: 0x60, // DRM_MODE_TYPE_DRIVER | DRM_MODE_TYPE_PREFERRED

        name,
    }
}
