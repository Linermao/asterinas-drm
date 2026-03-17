use alloc::{sync::Arc, vec::Vec};

use aster_gpu::drm::{
    DrmDevice, DrmError,
    drm_modes::DrmDisplayMode,
    gem::DrmGemObject,
    ioctl::DrmModeFbDirtyCmd,
    mode_config::DrmModeConfig,
    mode_object::{
        DrmObject,
        connector::{
            ConnectorState, ConnectorStatus, ConnectorType, DrmConnector, property::ConnectorProps,
        },
        crtc::{CrtcState, DrmCrtc},
        encoder::{DrmEncoder, EncoderState, EncoderType},
        framebuffer::DrmFramebuffer,
        plane::{DrmPlane, DrmPlaneType, PlaneState, property::PlaneProps},
        property::PropertyObject,
    },
};
use hashbrown::HashMap;
use ostd::sync::Mutex;

use crate::device::gpu::{VirtioGpuRect, device::GpuDevice, gem::VirtioGemObject};

pub fn virtio_gpu_output_init(scanout_id: u32, config: &mut DrmModeConfig) -> Result<(), DrmError> {
    let plane = Arc::new(VirtioGpuPlane::new(config, DrmPlaneType::Primary));
    let crtc = Arc::new(VirtioGpuCrtc::new(scanout_id, plane.clone(), None));
    let encoder = Arc::new(VirtioGpuEncoder::new(EncoderType::VIRTUAL));

    let connector_type = ConnectorType::VIRTUAL;
    let connector_type_id = config.next_connector_type_id(connector_type);
    let connector = Arc::new(VirtioGpuConnector::new(
        config,
        connector_type,
        connector_type_id,
    ));

    let _ = config.add_object(DrmObject::Plane(plane.clone()));
    let crtc_indices = config.add_object(DrmObject::Crtc(crtc.clone()));
    let encoder_indices = config.add_object(DrmObject::Encoder(encoder.clone()));
    let _ = config.add_object(DrmObject::Connector(connector.clone()));

    plane.set_possible_crtcs(&[crtc_indices]);
    encoder.set_possible_crtcs(&[crtc_indices]);
    connector.set_possible_encoders(&[encoder_indices]);

    Ok(())
}

#[derive(Debug)]
pub struct VirtioGpuPlane {
    state: Mutex<PlaneState>,
}

impl DrmPlane for VirtioGpuPlane {
    fn state(&self) -> &Mutex<PlaneState> {
        &self.state
    }
}

impl VirtioGpuPlane {
    fn new(config: &mut DrmModeConfig, type_: DrmPlaneType) -> Self {
        use PlaneProps::*;

        let mut properties: PropertyObject = HashMap::new();
        properties.insert(config.attach_property(&Type), type_ as u64);

        Self {
            state: Mutex::new(PlaneState::new(properties)),
        }
    }

    fn set_possible_crtcs(&self, crtc_indices: &[usize]) {
        self.state.lock().set_possible_crtcs(crtc_indices);
    }
}

#[derive(Debug)]
pub struct VirtioGpuCrtc {
    scanout_id: u32,
    state: Mutex<CrtcState>,
    primary: Arc<dyn DrmPlane>,
    cursor: Option<Arc<dyn DrmPlane>>,
}

impl DrmCrtc for VirtioGpuCrtc {
    fn state(&self) -> &Mutex<CrtcState> {
        &self.state
    }

    fn primary_plane(&self) -> &Arc<dyn DrmPlane> {
        &self.primary
    }

    fn cursor_plane(&self) -> &Option<Arc<dyn DrmPlane>> {
        &self.cursor
    }

    fn set_config(
        &self,
        x: u32,
        y: u32,
        fb: Arc<dyn DrmFramebuffer>,
        _connectors: Vec<Arc<dyn DrmConnector>>,
        dev: Arc<dyn DrmDevice>,
    ) -> Result<(), DrmError> {
        let vgpu = Arc::downcast::<GpuDevice>(dev).map_err(|_| DrmError::Invalid)?;
        let gem_object =
            Arc::downcast::<VirtioGemObject>(fb.gem_object()).map_err(|_| DrmError::Invalid)?;

        let width = fb.width();
        let height = fb.height();

        let rect = VirtioGpuRect {
            x,
            y,
            width,
            height,
        };
        let scanout_id = self.scanout_id;
        let resource_id = gem_object.resource_id();

        // ensure the host-side 2D resource sees the latest guest framebuffer contents
        // some devices (older qemu) may not implement this command; failure is
        // non‑fatal so we log it and continue with flush+scanout.
        vgpu.transfer_to_host_2d(resource_id, rect, 0)
            .map_err(|_| DrmError::Invalid)?;
        vgpu.set_scanout(scanout_id, resource_id, rect)
            .map_err(|_| DrmError::Invalid)?;
        // and finally flush to update the currently bound scanout
        vgpu.resource_flush(resource_id, rect)
            .map_err(|_| DrmError::Invalid)?;

        Ok(())
    }
}

impl VirtioGpuCrtc {
    fn new(scanout_id: u32, primary: Arc<dyn DrmPlane>, cursor: Option<Arc<dyn DrmPlane>>) -> Self {
        let properties: PropertyObject = HashMap::new();

        Self {
            scanout_id,
            state: Mutex::new(CrtcState::new(properties)),
            primary,
            cursor,
        }
    }
}

#[derive(Debug)]
pub struct VirtioGpuEncoder {
    type_: EncoderType,
    state: Mutex<EncoderState>,
}

impl DrmEncoder for VirtioGpuEncoder {
    fn state(&self) -> &Mutex<EncoderState> {
        &self.state
    }

    fn type_(&self) -> EncoderType {
        self.type_
    }
}

impl VirtioGpuEncoder {
    fn new(type_: EncoderType) -> Self {
        Self {
            type_,
            state: Mutex::new(EncoderState::new()),
        }
    }

    fn set_possible_crtcs(&self, crtc_indices: &[usize]) {
        self.state.lock().set_possible_crtcs(crtc_indices);
    }
}

#[derive(Debug)]
pub struct VirtioGpuConnector {
    type_: ConnectorType,
    type_id_: u32,
    state: Mutex<ConnectorState>,
}

impl DrmConnector for VirtioGpuConnector {
    fn state(&self) -> &Mutex<ConnectorState> {
        &self.state
    }

    fn type_(&self) -> ConnectorType {
        self.type_
    }

    fn type_id_(&self) -> u32 {
        self.type_id_
    }

    fn detect(&self) -> Result<ConnectorStatus, DrmError> {
        // TODO
        Ok(ConnectorStatus::Connected)
    }

    fn fill_modes(&self, dev: Arc<dyn DrmDevice>) -> Result<(), DrmError> {
        {
            let mut state = self.state.lock();
            // TODO: with force
            let status = self.detect()?;
            state.set_status(status);
        }

        self.get_modes(dev)?;

        Ok(())
    }

    fn get_modes(&self, dev: Arc<dyn DrmDevice>) -> Result<(), DrmError> {
        let vgpu = Arc::downcast::<GpuDevice>(dev).map_err(|_| DrmError::Invalid)?;
        let display_infos = vgpu.display_infos();

        if vgpu.has_edid() {
            // TODO
        }

        // TODO
        let mode = if let Some(info) = display_infos.first() {
            if info.enabled != 0 && info.rect.width > 0 && info.rect.height > 0 {
                DrmDisplayMode::from_resolution(info.rect.width as u16, info.rect.height as u16)
            } else {
                DrmDisplayMode::default()
            }
        } else {
            DrmDisplayMode::default()
        };

        let mut state = self.state.lock();
        state.set_modes(&[mode]);

        Ok(())
    }
}

impl VirtioGpuConnector {
    fn new(config: &mut DrmModeConfig, type_: ConnectorType, type_id_: u32) -> Self {
        use ConnectorProps::*;

        let mut properties: PropertyObject = HashMap::new();

        properties.insert(config.attach_property(&DPMS), 0);
        properties.insert(config.attach_property(&LinkStatus), 0);
        properties.insert(config.attach_property(&NonDesktop), 0);
        properties.insert(config.attach_property(&TILE), 0);

        Self {
            type_,
            type_id_,
            state: Mutex::new(ConnectorState::new(properties)),
        }
    }

    fn set_possible_encoders(&self, encoder_indices: &[usize]) {
        self.state.lock().set_possible_encoders(encoder_indices);
    }
}

#[derive(Debug)]
pub struct VirtioGpuFramebuffer {
    width: u32,
    height: u32,
    gem_object: Arc<dyn DrmGemObject>,
}

impl DrmFramebuffer for VirtioGpuFramebuffer {
    fn gem_object(&self) -> Arc<dyn DrmGemObject> {
        self.gem_object.clone()
    }

    fn width(&self) -> u32 {
        self.width
    }

    fn height(&self) -> u32 {
        self.height
    }

    fn dirty(
        &self,
        dev: Arc<dyn DrmDevice>,
        _dirty_cmd: &DrmModeFbDirtyCmd,
    ) -> Result<(), DrmError> {
        let vgpu = Arc::downcast::<GpuDevice>(dev).map_err(|_| DrmError::Invalid)?;
        let gem_object =
            Arc::downcast::<VirtioGemObject>(self.gem_object()).map_err(|_| DrmError::Invalid)?;

        let width = self.width();
        let height = self.height();

        // TODO
        let rect = VirtioGpuRect {
            x: 0,
            y: 0,
            width,
            height,
        };
        let resource_id = gem_object.resource_id();

        log::error!("[kernel] correct running!");

        vgpu.transfer_to_host_2d(resource_id, rect, 0)
            .map_err(|_| DrmError::Invalid)?;
        vgpu.resource_flush(resource_id, rect)
            .map_err(|_| DrmError::Invalid)?;

        Ok(())
    }
}

impl VirtioGpuFramebuffer {
    pub fn new(width: u32, height: u32, gem_object: Arc<dyn DrmGemObject>) -> Self {
        Self {
            width,
            height,
            gem_object,
        }
    }
}
