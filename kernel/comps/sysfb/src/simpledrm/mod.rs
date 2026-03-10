use alloc::sync::Arc;

use aster_gpu::drm::{
    DrmDevice, DrmError, DrmFeatures,
    drm_modes::DrmDisplayMode,
    mode_config::DrmModeConfig,
    mode_object::{
        DrmObject,
        connector::{
            ConnectorState, ConnectorStatus, ConnectorType, DrmConnector, property::ConnectorProps,
        },
        crtc::{CrtcState, DrmCrtc},
        encoder::{DrmEncoder, EncoderState, EncoderType},
        plane::{DrmPlane, PlaneState},
        property::PropertyObject,
    },
};
use hashbrown::HashMap;
use ostd::{boot::boot_info, sync::Mutex};

const SIMPLEDRM_FEATURES: DrmFeatures =
    DrmFeatures::from_bits_truncate(DrmFeatures::GEM.bits() | DrmFeatures::MODESET.bits());
const SIMPLEDRM_NAME: &'static str = "simpledrm";
const SIMPLEDRM_DESC: &'static str = "DRM driver for simple-framebuffer platform devices";
const SIMPLEDRM_DATE: &'static str = "0";

const MIN_WIDTH: u32 = 1;
const MAX_WIDTH: u32 = 4096;
const MIN_HEIGHT: u32 = 1;
const MAX_HEIGHT: u32 = 4096;

#[derive(Debug)]
pub(crate) struct SimpleDrmDevice {
    mode_config: Mutex<DrmModeConfig>,
}

impl SimpleDrmDevice {
    pub fn new() -> Self {
        let mut mode_config = DrmModeConfig::new(MIN_WIDTH, MAX_WIDTH, MIN_HEIGHT, MAX_HEIGHT);

        Self::init_objects(&mut mode_config);

        Self {
            mode_config: Mutex::new(mode_config),
        }
    }

    fn init_objects(config: &mut DrmModeConfig) {
        let plane = Arc::new(SimpleDrmPlane::new());
        let crtc = Arc::new(SimpleDrmCrtc::new(plane.clone(), None));
        let encoder = Arc::new(SimpleDrmEncoder::new(EncoderType::VIRTUAL));

        let connector_type = ConnectorType::VIRTUAL;
        let connector_type_id = config.next_connector_type_id(connector_type);
        let connector = Arc::new(SimpleDrmConnector::new(
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
    }
}

impl DrmDevice for SimpleDrmDevice {
    fn name(&self) -> &str {
        SIMPLEDRM_NAME
    }

    fn desc(&self) -> &str {
        SIMPLEDRM_DESC
    }

    fn date(&self) -> &str {
        SIMPLEDRM_DATE
    }

    fn features(&self) -> DrmFeatures {
        SIMPLEDRM_FEATURES
    }

    fn mode_config(&self) -> &Mutex<DrmModeConfig> {
        &self.mode_config
    }
}

#[derive(Debug)]
struct SimpleDrmPlane {
    state: Mutex<PlaneState>,
}

impl DrmPlane for SimpleDrmPlane {
    fn state(&self) -> &Mutex<PlaneState> {
        &self.state
    }
}

impl SimpleDrmPlane {
    fn new() -> Self {
        let properties: PropertyObject = HashMap::new();
        Self {
            state: Mutex::new(PlaneState::new(properties)),
        }
    }

    fn set_possible_crtcs(&self, crtc_indices: &[usize]) {
        self.state.lock().set_possible_crtcs(crtc_indices);
    }
}

#[derive(Debug)]
struct SimpleDrmCrtc {
    state: Mutex<CrtcState>,
    primary: Arc<dyn DrmPlane>,
    cursor: Option<Arc<dyn DrmPlane>>,
}

impl DrmCrtc for SimpleDrmCrtc {
    fn state(&self) -> &Mutex<CrtcState> {
        &self.state
    }

    fn primary_plane(&self) -> Arc<dyn DrmPlane> {
        self.primary.clone()
    }

    fn cursor_plane(&self) -> Option<Arc<dyn DrmPlane>> {
        self.cursor.clone()
    }
}

impl SimpleDrmCrtc {
    fn new(primary: Arc<dyn DrmPlane>, cursor: Option<Arc<dyn DrmPlane>>) -> Self {
        let properties: PropertyObject = HashMap::new();

        Self {
            state: Mutex::new(CrtcState::new(properties)),
            primary,
            cursor,
        }
    }
}

#[derive(Debug)]
struct SimpleDrmEncoder {
    type_: EncoderType,
    state: Mutex<EncoderState>,
}

impl DrmEncoder for SimpleDrmEncoder {
    fn type_(&self) -> EncoderType {
        self.type_
    }

    fn state(&self) -> &Mutex<EncoderState> {
        &self.state
    }
}

impl SimpleDrmEncoder {
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
struct SimpleDrmConnector {
    type_: ConnectorType,
    type_id_: u32,
    state: Mutex<ConnectorState>,
}

impl DrmConnector for SimpleDrmConnector {
    fn type_(&self) -> ConnectorType {
        self.type_
    }

    fn type_id_(&self) -> u32 {
        self.type_id_
    }

    fn state(&self) -> &Mutex<ConnectorState> {
        &self.state
    }

    fn fill_modes(&self) -> Result<(), DrmError> {
        {
            let mut state = self.state.lock();
            // TODO: with force
            let status = self.detect()?;
            state.set_status(status);
        }

        self.get_modes()?;

        Ok(())
    }

    fn get_modes(&self) -> Result<(), DrmError> {
        let fb_info = boot_info().framebuffer_arg;

        let mode = if let Some(fb) = fb_info {
            // Create mode from framebuffer resolution
            DrmDisplayMode::from_resolution(fb.width as u16, fb.height as u16)
        } else {
            // Fallback to default mode if no framebuffer info
            DrmDisplayMode::default()
        };

        let mut state = self.state.lock();
        state.set_modes(&[mode]);

        Ok(())
    }

    fn detect(&self) -> Result<ConnectorStatus, DrmError> {
        Ok(ConnectorStatus::Connected)
    }
}

impl SimpleDrmConnector {
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
