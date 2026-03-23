use alloc::{sync::Arc, vec::Vec};

use aster_gpu::drm::{
    DrmDevice, DrmError,
    atomic::{DrmAtomicHelper, vblank::DrmVblankState},
    drm_modes::DrmDisplayMode,
    gem::DrmGemObject,
    ioctl::DrmModeFbDirtyCmd,
    mode_config::{DrmModeConfig, ObjectId},
    mode_object::{
        DrmObject,
        connector::{
            ConnectorState, ConnectorStatus, ConnectorType, DrmConnector, property::ConnectorProps,
        },
        crtc::{CrtcState, DrmCrtc, property::CrtcProps},
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
    let primary_plane = Arc::new(VirtioGpuPlane::new(config, DrmPlaneType::Primary));
    let (primary_id, _) = config.add_object(DrmObject::Plane(primary_plane.clone()));

    let crtc = Arc::new(VirtioGpuCrtc::new(
        config,
        scanout_id,
        primary_plane.clone(),
        primary_id,
        None,
    ));
    let encoder = Arc::new(VirtioGpuEncoder::new(EncoderType::VIRTUAL));

    let connector_type = ConnectorType::VIRTUAL;
    let connector_type_id = config.next_connector_type_id(connector_type);
    let connector = Arc::new(VirtioGpuConnector::new(
        config,
        connector_type,
        connector_type_id,
    ));

    let (_, crtc_indices) = config.add_object(DrmObject::Crtc(crtc.clone()));
    let (_, encoder_indices) = config.add_object(DrmObject::Encoder(encoder.clone()));
    let _ = config.add_object(DrmObject::Connector(connector.clone()));

    primary_plane.set_possible_crtcs(&[crtc_indices]);
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
        properties.insert(config.attach_property(&CrtcId), 0);
        properties.insert(config.attach_property(&FbId), 0);

        properties.insert(config.attach_property(&SrcX), 0);
        properties.insert(config.attach_property(&SrcY), 0);
        properties.insert(config.attach_property(&SrcW), 0);
        properties.insert(config.attach_property(&SrcH), 0);
        properties.insert(config.attach_property(&CrtcX), 0);
        properties.insert(config.attach_property(&CrtcY), 0);
        properties.insert(config.attach_property(&CrtcW), 0);
        properties.insert(config.attach_property(&CrtcH), 0);

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
    vblank_state: Mutex<DrmVblankState>,
    primary: Arc<dyn DrmPlane>,
    primary_id: ObjectId,
    cursor: Option<Arc<dyn DrmPlane>>,
}

impl DrmCrtc for VirtioGpuCrtc {
    fn state(&self) -> &Mutex<CrtcState> {
        &self.state
    }

    fn vblank_state(&self) -> &Mutex<DrmVblankState> {
        &self.vblank_state
    }

    fn primary_plane(&self) -> &Arc<dyn DrmPlane> {
        &self.primary
    }

    fn primary_plane_id(&self) -> ObjectId {
        self.primary_id
    }

    fn cursor_plane(&self) -> &Option<Arc<dyn DrmPlane>> {
        &self.cursor
    }
}

impl VirtioGpuCrtc {
    fn new(
        config: &mut DrmModeConfig,
        scanout_id: u32,
        primary: Arc<dyn DrmPlane>,
        primary_id: ObjectId,
        cursor: Option<Arc<dyn DrmPlane>>,
    ) -> Self {
        use CrtcProps::*;

        let mut properties: PropertyObject = HashMap::new();
        properties.insert(config.attach_property(&Active), 0);
        properties.insert(config.attach_property(&ModeId), 0);

        Self {
            scanout_id,
            state: Mutex::new(CrtcState::new(properties)),
            vblank_state: Mutex::new(DrmVblankState::new()),
            primary,
            primary_id,
            cursor,
        }
    }

    pub fn scanout_id(&self) -> u32 {
        self.scanout_id
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

        properties.insert(config.attach_property(&CrtcId), 0);
        properties.insert(config.attach_property(&DPMS), 0);
        properties.insert(config.attach_property(&LinkStatus), 0);
        properties.insert(config.attach_property(&NonDesktop), 0);
        properties.insert(config.attach_property(&Tile), 0);

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
