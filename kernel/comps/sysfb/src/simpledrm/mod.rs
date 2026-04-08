use alloc::{boxed::Box, sync::Arc, vec::Vec};

use aster_framebuffer::FRAMEBUFFER;
use aster_gpu::drm::{
    DrmDevice, DrmDeviceCaps, DrmError, DrmFeatures,
    atomic::{DrmAtomicOps, DrmAtomicPendingState, helper::DrmAtomicHelper},
    drm_modes::DrmDisplayMode,
    gem::{DrmGemBackend, DrmGemObject, DrmGemOps, MemfdAllocatorType},
    ioctl::{DrmIoctlOps, DrmModeCreateDumb, DrmModeCrtc, DrmModeCrtcPageFlip, DrmModeFbCmd2},
    kms::{
        DrmKmsOps,
        vblank::{DrmVblankState, PageFlipEvent, VblankCallback},
    },
    objects::{
        DrmObject, DrmObjects, ObjectId,
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
use ostd::{boot::boot_info, mm::io_util::HasVmReaderWriter, sync::Mutex};

const SIMPLEDRM_NAME: &'static str = "simpledrm";
const SIMPLEDRM_DESC: &'static str = "DRM driver for simple-framebuffer platform devices";
const SIMPLEDRM_DATE: &'static str = "0";

const MIN_WIDTH: u32 = 1;
const MAX_WIDTH: u32 = 4096;
const MIN_HEIGHT: u32 = 1;
const MAX_HEIGHT: u32 = 4096;
const PREFERRED_DEPTH: u32 = 16;

#[derive(Debug)]
pub(crate) struct SimpleDrmDevice {
    objects: Mutex<DrmObjects>,
}

impl SimpleDrmDevice {
    pub fn new() -> Self {
        let mut objects = DrmObjects::new();

        let primary = Arc::new(SimpleDrmPlane::new(&mut objects, DrmPlaneType::Primary));
        let (primary_id, _) = objects.add_object(DrmObject::Plane(primary.clone()));

        let crtc = Arc::new(SimpleDrmCrtc::new(
            &mut objects,
            primary.clone(),
            primary_id,
            None,
        ));
        let (_, crtc_indices) = objects.add_object(DrmObject::Crtc(crtc.clone()));

        let encoder = Arc::new(SimpleDrmEncoder::new(EncoderType::VIRTUAL));
        let (_, encoder_indices) = objects.add_object(DrmObject::Encoder(encoder.clone()));

        let connector_type = ConnectorType::VIRTUAL;
        let connector_type_id = objects.next_connector_type_id(connector_type);
        let connector = Arc::new(SimpleDrmConnector::new(
            &mut objects,
            connector_type,
            connector_type_id,
        ));
        let _ = objects.add_object(DrmObject::Connector(connector.clone()));

        primary.set_possible_crtcs(&[crtc_indices]);
        encoder.set_possible_crtcs(&[crtc_indices]);
        connector.set_possible_encoders(&[encoder_indices]);

        Self {
            objects: Mutex::new(objects),
        }
    }
}

impl DrmKmsOps for SimpleDrmDevice {
    fn set_crtc(
        &self,
        crtc_resp: &DrmModeCrtc,
        connector_ids: Vec<ObjectId>,
    ) -> Result<(), DrmError> {
        self.atomic_helper_set_config(crtc_resp, connector_ids)
    }

    fn page_flip(
        &self,
        page_flip: &DrmModeCrtcPageFlip,
        vblank_callback: Arc<dyn VblankCallback>,
        target: Option<u32>,
    ) -> Result<(), DrmError> {
        self.atomic_helper_pageflip(page_flip, vblank_callback, target)
    }

    fn dirty_framebuffer(&self, fb_id: ObjectId) -> Result<(), DrmError> {
        self.atomic_helper_dirty_framebuffer(fb_id)
    }

    fn update_connector_modes_and_status(&self, connector_id: ObjectId) -> Result<(), DrmError> {
        let objects = self.objects.lock();
        let connector = objects
            .get_object_by_id::<dyn DrmConnector>(connector_id)
            .ok_or(DrmError::NotFound)?;

        // TODO:
        let status = ConnectorStatus::Connected;
        let fb_info = boot_info().framebuffer_arg;
        let mode = if let Some(fb) = fb_info {
            // Create mode from framebuffer resolution
            DrmDisplayMode::from_resolution(fb.width as u16, fb.height as u16)
        } else {
            // Fallback to default mode if no framebuffer info
            DrmDisplayMode::default()
        };

        let mut state = connector.state().lock();
        state.set_status(status);
        state.set_modes(&[mode]);

        Ok(())
    }
}

impl DrmAtomicOps for SimpleDrmDevice {
    fn atomic_commit(
        &self,
        nonblock: bool,
        pending_state: &mut DrmAtomicPendingState,
        page_flip_event: Option<PageFlipEvent>,
    ) -> Result<(), DrmError> {
        self.atomic_helper_commit(nonblock, pending_state, page_flip_event)
    }

    fn atomic_commit_tail(
        &self,
        pending_state: &mut DrmAtomicPendingState,
    ) -> Result<(), DrmError> {
        self.atomic_helper_commit_tail(pending_state)
    }
}

impl DrmGemOps for SimpleDrmDevice {
    fn create_dumb(
        &self,
        args: &DrmModeCreateDumb,
        memfd_allocator_fn: MemfdAllocatorType,
    ) -> Result<Arc<dyn DrmGemObject>, DrmError> {
        let pitch = args.width * (args.bpp / 8);
        let size = (pitch * args.height) as u64;

        let backend = memfd_allocator_fn("simpledrm-dumb", size)?;
        let gem_object = SimpleDrmGemObject::new(pitch, size, backend);

        Ok(Arc::new(gem_object))
    }

    fn fb_create(
        &self,
        fb_cmd: &DrmModeFbCmd2,
        gem_object: Arc<dyn DrmGemObject>,
    ) -> Result<ObjectId, DrmError> {
        let fb = Arc::new(SimpleDrmFramebuffer::new(
            fb_cmd.width,
            fb_cmd.height,
            gem_object,
        ));

        let handle = self.objects.lock().add_framebuffer(fb);

        Ok(handle)
    }
}

impl DrmIoctlOps for SimpleDrmDevice {
    
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
        DrmFeatures::GEM | DrmFeatures::MODESET | DrmFeatures::ATOMIC
    }

    fn capbilities(&self) -> DrmDeviceCaps {
        DrmDeviceCaps::DUMB_CREATE
    }

    fn min_width(&self) -> u32 {
        MIN_WIDTH
    }

    fn max_width(&self) -> u32 {
        MAX_WIDTH
    }

    fn min_height(&self) -> u32 {
        MIN_HEIGHT
    }

    fn max_height(&self) -> u32 {
        MAX_HEIGHT
    }

    fn preferred_depth(&self) -> u32 {
        PREFERRED_DEPTH
    }

    fn prefer_shadow(&self) -> u32 {
        0
    }

    fn cursor_width(&self) -> u32 {
        0
    }

    fn cursor_height(&self) -> u32 {
        0
    }

    fn support_async_page_flip(&self) -> bool {
        true
    }

    fn support_fb_modifiers(&self) -> bool {
        false
    }

    fn objects(&self) -> &Mutex<DrmObjects> {
        &self.objects
    }
}

impl DrmAtomicHelper for SimpleDrmDevice {
    fn atomic_flush(&self, crtc_id: ObjectId) -> Result<(), DrmError> {
        let objects = self.objects.lock();

        let crtc = objects
            .get_object_by_id::<dyn DrmCrtc>(crtc_id)
            .ok_or(DrmError::NotFound)?;

        let fb_id = crtc.primary_plane().fb_id().ok_or(DrmError::NotFound)?;
        let fb = objects
            .get_object_by_id::<dyn DrmFramebuffer>(fb_id)
            .ok_or(DrmError::NotFound)?;

        let Some(framebuffer) = FRAMEBUFFER.get() else {
            return Err(DrmError::NotFound);
        };

        let iomem = framebuffer.io_mem();
        let mut writer = iomem.writer().to_fallible();
        fb.gem_object().backend().read(0, &mut writer)?;

        // TODO: simpledrm use vblank_timer to set 60hz vblank signal
        crtc.handle_vblank()?;

        Ok(())
    }
}

#[derive(Debug)]
struct SimpleDrmGemObject {
    pitch: u32,
    size: u64,
    backend: Box<dyn DrmGemBackend>,
}

impl DrmGemObject for SimpleDrmGemObject {
    fn pitch(&self) -> u32 {
        self.pitch
    }

    fn size(&self) -> u64 {
        self.size
    }

    fn backend(&self) -> &Box<dyn DrmGemBackend> {
        &self.backend
    }
}

impl SimpleDrmGemObject {
    fn new(pitch: u32, size: u64, backend: Box<dyn DrmGemBackend>) -> Self {
        Self {
            pitch,
            size,
            backend,
        }
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
    fn new(objects: &mut DrmObjects, type_: DrmPlaneType) -> Self {
        use PlaneProps::*;
        let mut properties: PropertyObject = HashMap::new();

        properties.insert(objects.attach_property(&Type), type_ as u64);
        properties.insert(objects.attach_property(&CrtcId), 0);
        properties.insert(objects.attach_property(&FbId), 0);

        properties.insert(objects.attach_property(&SrcX), 0);
        properties.insert(objects.attach_property(&SrcY), 0);
        properties.insert(objects.attach_property(&SrcW), 0);
        properties.insert(objects.attach_property(&SrcH), 0);
        properties.insert(objects.attach_property(&CrtcX), 0);
        properties.insert(objects.attach_property(&CrtcY), 0);
        properties.insert(objects.attach_property(&CrtcW), 0);
        properties.insert(objects.attach_property(&CrtcH), 0);

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
    vblank_state: Mutex<DrmVblankState>,
    primary: Arc<dyn DrmPlane>,
    primary_id: ObjectId,
    cursor: Option<Arc<dyn DrmPlane>>,
}

impl DrmCrtc for SimpleDrmCrtc {
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

impl SimpleDrmCrtc {
    fn new(
        objects: &mut DrmObjects,
        primary: Arc<dyn DrmPlane>,
        primary_id: ObjectId,
        cursor: Option<Arc<dyn DrmPlane>>,
    ) -> Self {
        use CrtcProps::*;

        let mut properties: PropertyObject = HashMap::new();
        properties.insert(objects.attach_property(&Active), 0);
        properties.insert(objects.attach_property(&ModeId), 0);

        Self {
            state: Mutex::new(CrtcState::new(properties)),
            vblank_state: Mutex::new(DrmVblankState::new()),
            primary,
            primary_id,
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
}

impl SimpleDrmConnector {
    fn new(objects: &mut DrmObjects, type_: ConnectorType, type_id_: u32) -> Self {
        use ConnectorProps::*;

        let mut properties: PropertyObject = HashMap::new();

        properties.insert(objects.attach_property(&CrtcId), 0);
        properties.insert(objects.attach_property(&DPMS), 0);
        properties.insert(objects.attach_property(&LinkStatus), 0);
        properties.insert(objects.attach_property(&NonDesktop), 0);
        properties.insert(objects.attach_property(&Tile), 0);

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
struct SimpleDrmFramebuffer {
    width: u32,
    height: u32,
    gem_object: Arc<dyn DrmGemObject>,
}

impl DrmFramebuffer for SimpleDrmFramebuffer {
    fn width(&self) -> u32 {
        self.width
    }

    fn height(&self) -> u32 {
        self.height
    }

    fn gem_object(&self) -> Arc<dyn DrmGemObject> {
        self.gem_object.clone()
    }
}

impl SimpleDrmFramebuffer {
    fn new(width: u32, height: u32, gem_object: Arc<dyn DrmGemObject>) -> Self {
        Self {
            width,
            height,
            gem_object,
        }
    }
}
