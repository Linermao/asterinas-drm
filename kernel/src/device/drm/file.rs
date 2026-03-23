use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use aster_gpu::drm::{
    DrmDevice, DrmDeviceCaps, DrmFeatures,
    atomic::{
        DrmAtomicFlags, DrmAtomicPendingState,
        vblank::{
            DRM_EVENT_VBLANK_LEN, DrmPendingVblankEvent, DrmVblankEvent, PageFlipEvent,
            VblankCallback,
        },
    },
    drm_modes::DrmModeModeInfo,
    gem::DrmGemObject,
    ioctl::*,
    mode_object::{
        DrmObjectType,
        connector::DrmConnector,
        crtc::DrmCrtc,
        encoder::DrmEncoder,
        framebuffer::DrmFramebuffer,
        plane::DrmPlane,
        property::{DrmModeBlob, DrmProperty, DrmPropertyKind, PropertyEnum},
    },
};
use hashbrown::HashMap;
use ostd::mm::VmIo;

use crate::{
    current_userspace,
    device::drm::{
        ioctl_defs::*,
        memfd::{DrmMemFdFile, memfd_allocator},
        minor::DrmMinor,
    },
    dispatch_drm_ioctl,
    events::IoEvents,
    fs::{
        file_handle::Mappable,
        inode_handle::FileIo,
        utils::{InodeIo, StatusFlags},
    },
    prelude::*,
    process::signal::{PollHandle, Pollable, Pollee},
    util::ioctl::RawIoctl,
};

struct DrmFileVblank {
    event_pollee: Pollee,
    event_queue: Mutex<VecDeque<Vec<u8>>>,
}

impl DrmFileVblank {
    fn new() -> Self {
        Self {
            event_pollee: Pollee::new(),
            event_queue: Mutex::new(VecDeque::new()),
        }
    }
}

impl VblankCallback for DrmFileVblank {
    fn send_vblank_event(&self, bytes: &[u8]) {
        self.event_queue.lock().push_back(bytes.to_vec());
        self.event_pollee.notify(IoEvents::IN);
    }
}

pub(super) struct DrmFile {
    minor: Arc<DrmMinor>,
    next_gem_handle: AtomicU32,
    gem_table: Mutex<HashMap<u32, Arc<dyn DrmGemObject>>>,

    /// True when the client has asked us to expose stereo 3D mode flags.
    stereo_allowed: AtomicBool,
    /// True if client understands CRTC primary planes and cursor planes
    /// in the plane list. Automatically set when atomic is set.
    universal_planes: AtomicBool,
    /// True if client understands atomic properties.
    atomic: AtomicBool,
    /// True, if client can handle picture aspect ratios, and has requested
    /// to pass this information along with the mode.
    aspect_ratio_allowed: AtomicBool,
    /// True if client understands writeback connectors
    writeback_connectors: AtomicBool,
    /// This client is capable of handling the cursor plane with the
    /// restrictions imposed on it by the virtualized drivers.
    supports_virtualized_cursor_plane: AtomicBool,

    vblank: Arc<DrmFileVblank>,
}

impl DrmFile {
    pub fn new(minor: Arc<DrmMinor>) -> Self {
        Self {
            minor,
            next_gem_handle: AtomicU32::new(1),
            gem_table: Mutex::new(HashMap::new()),

            stereo_allowed: AtomicBool::new(false),
            universal_planes: AtomicBool::new(false),
            atomic: AtomicBool::new(false),
            aspect_ratio_allowed: AtomicBool::new(false),
            writeback_connectors: AtomicBool::new(false),
            supports_virtualized_cursor_plane: AtomicBool::new(false),
            vblank: Arc::new(DrmFileVblank::new()),
        }
    }

    pub fn device(&self) -> Arc<dyn DrmDevice> {
        self.minor.device()
    }

    pub fn next_gem_handle(&self) -> u32 {
        self.next_gem_handle.fetch_add(1, Ordering::SeqCst)
    }

    pub fn add_gem(&self, id: u32, gem: Arc<dyn DrmGemObject>) {
        self.gem_table.lock().insert(id, gem);
    }

    pub fn lookup_gem(&self, id: u32) -> Option<Arc<dyn DrmGemObject>> {
        self.gem_table.lock().get(&id).cloned()
    }

    pub fn remove_gem(&self, id: u32) -> Option<Arc<dyn DrmGemObject>> {
        self.gem_table.lock().remove(&id)
    }
}

impl Pollable for DrmFile {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.vblank.event_pollee.poll_with(mask, poller, || {
            let mut events = IoEvents::OUT;
            if !self.vblank.event_queue.lock().is_empty() {
                events |= IoEvents::IN;
            }
            events
        })
    }
}

impl InodeIo for DrmFile {
    fn read_at(
        &self,
        _offset: usize,
        writer: &mut VmWriter,
        status_flags: StatusFlags,
    ) -> Result<usize> {
        let is_nonblocking = status_flags.contains(StatusFlags::O_NONBLOCK);
        let mut queue = self.vblank.event_queue.lock();

        if queue.is_empty() {
            if is_nonblocking {
                return_errno!(Errno::EAGAIN);
            }
            // Blocking DRM read should wait for events; current fallback is EAGAIN.
            return_errno!(Errno::EAGAIN);
        }

        let mut total_written = 0usize;
        while let Some(event) = queue.front() {
            if event.len() > writer.avail() {
                if total_written == 0 {
                    // Linux DRM requires user buffer to fit the next full event.
                    return_errno!(Errno::EINVAL);
                }
                break;
            }

            let Some(event) = queue.pop_front() else {
                break;
            };
            writer.write_fallible(&mut event.as_slice().into())?;
            total_written += event.len();
        }

        if queue.is_empty() {
            self.vblank.event_pollee.invalidate();
        }

        Ok(total_written)
    }

    fn write_at(
        &self,
        _offset: usize,
        _reader: &mut VmReader,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "drm: write not supported");
    }
}

impl FileIo for DrmFile {
    fn check_seekable(&self) -> Result<()> {
        Ok(())
    }

    fn is_offset_aware(&self) -> bool {
        true
    }

    fn mappable_with_offset(&self, offset: usize) -> Result<Mappable> {
        if let Some(handle) = self.device().lookup_gem_handle(offset) {
            if let Some(gem_obj) = self.lookup_gem(handle) {
                if let Some(memfd) = gem_obj.backend().downcast_ref::<DrmMemFdFile>() {
                    return memfd.mappable();
                } else {
                    // TODO
                }
            }
        }

        return_errno!(Errno::ENOENT);
    }

    fn ioctl(&self, raw_ioctl: RawIoctl) -> Result<i32> {
        dispatch_drm_ioctl!(
            self,
            match raw_ioctl {
                cmd @ DrmIoctlVersion => {
                    let mut version: DrmVersion = cmd.read()?;

                    let dev = self.device();

                    let name = dev.name();
                    let name_len = name.len() + 1;
                    let desc = dev.desc();
                    let desc_len = desc.len() + 1;
                    let date = dev.date();
                    let date_len = date.len() + 1;

                    if version.is_first_call() {
                        version.name_len = name_len;
                        version.desc_len = desc_len;
                        version.date_len = date_len;

                        cmd.write(&version)?;
                    } else {
                        // TODO: better write cstring method
                        // the name,desc,date now is u64, maybe should use cstring?
                        if version.name_len >= name_len {
                            current_userspace!()
                                .write_bytes(version.name as usize, name.as_bytes())?;
                        } else {
                            return_errno!(Errno::EINVAL);
                        }

                        if version.desc_len >= desc_len {
                            current_userspace!()
                                .write_bytes(version.desc as usize, desc.as_bytes())?;
                        } else {
                            return_errno!(Errno::EINVAL);
                        }

                        if version.date_len >= date_len {
                            current_userspace!()
                                .write_bytes(version.date as usize, date.as_bytes())?;
                        } else {
                            return_errno!(Errno::EINVAL);
                        }

                        // TODO: major and minor
                    }

                    Ok(0)
                }
                cmd @ DrmIoctlGetCap => {
                    use DrmGetCapabilities::*;

                    let mut req: DrmGetCap = cmd.read()?;
                    let cap = DrmGetCapabilities::try_from(req.capability)?;
                    let dev = self.device();

                    let value = match cap {
                        TimestampMonotonic => 1,
                        Prime => (DrmPrimeValue::IMPORT | DrmPrimeValue::EXPORT).bits(),
                        SyncObj => dev.check_feature(DrmFeatures::SYNCOBJ) as u64,
                        SyncObjTimeline => dev.check_feature(DrmFeatures::SYNCOBJ_TIMELINE) as u64,
                        _ => {
                            if !dev.check_feature(DrmFeatures::MODESET) {
                                return_errno!(Errno::EOPNOTSUPP);
                            }

                            let mode_config = dev.mode_config().lock();
                            match cap {
                                DumbBuffer => {
                                    dev.check_capbility(DrmDeviceCaps::DUMB_CREATE) as u64
                                }
                                VblankHighCrtc => 1,
                                DumbPreferredDepth => mode_config.preferred_depth() as u64,
                                DumbPreferShadow => mode_config.prefer_shadow() as u64,
                                AsyncPageFlip => mode_config.allow_async_page_flip() as u64,
                                PageFlipTarget => {
                                    // TODO: check if each crtc has func: page_flip_target
                                    0
                                }
                                CursorWidth => match mode_config.cursor_width() {
                                    0 => 64,
                                    w => w as u64,
                                },
                                CursorHeight => match mode_config.cursor_height() {
                                    0 => 64,
                                    h => h as u64,
                                },
                                Addfb2Modifiers => !mode_config.fb_modifiers_not_supported() as u64,
                                CrtcInVblankEvent => 1,
                                AtomicAsyncPageFlip => {
                                    (dev.check_feature(DrmFeatures::ATOMIC)
                                        && mode_config.allow_async_page_flip())
                                        as u64
                                }
                                _ => 0,
                            }
                        }
                    };

                    req.value = value;

                    cmd.write(&req)?;
                    Ok(0)
                }
                cmd @ DrmIoctlSetClientCap => {
                    if !self.device().check_feature(DrmFeatures::MODESET) {
                        return_errno!(Errno::EOPNOTSUPP);
                    }

                    let req: DrmSetClientCap = cmd.read()?;

                    match DrmSetCapabilities::try_from(req.capability)? {
                        DrmSetCapabilities::Stereo3D => match req.value {
                            0 | 1 => {
                                self.stereo_allowed.store(req.value == 1, Ordering::Relaxed);
                            }
                            _ => return_errno!(Errno::EINVAL),
                        },
                        DrmSetCapabilities::UniversalPlane => {
                            match req.value {
                                0 | 1 => {
                                    self.universal_planes
                                        .store(req.value == 1, Ordering::Relaxed);
                                }
                                _ => return_errno!(Errno::EINVAL),
                            };
                        }
                        DrmSetCapabilities::Atomic => {
                            if !self.device().check_feature(DrmFeatures::ATOMIC) {
                                return_errno!(Errno::EOPNOTSUPP);
                            }
                            // TODO: The modesetting DDX has a totally broken idea of atomic.
                            // if (current->comm[0] == 'X' && req->value == 1) {
                            // 	pr_info("broken atomic modeset userspace detected, disabling atomic\n");
                            //  return -EOPNOTSUPP;
                            // }

                            match req.value {
                                0 | 1 | 2 => {
                                    let v = req.value;

                                    self.atomic.store(v >= 1, Ordering::Relaxed);
                                    self.universal_planes.store(v >= 1, Ordering::Relaxed);
                                    self.aspect_ratio_allowed.store(v == 2, Ordering::Relaxed);
                                }
                                _ => return_errno!(Errno::EINVAL),
                            }
                        }
                        DrmSetCapabilities::AspectRatio => {
                            match req.value {
                                0 | 1 => {
                                    self.aspect_ratio_allowed
                                        .store(req.value == 1, Ordering::Relaxed);
                                }
                                _ => return_errno!(Errno::EINVAL),
                            };
                        }
                        DrmSetCapabilities::WritebackConnectors => {
                            if !self.atomic.load(Ordering::Relaxed) {
                                return_errno!(Errno::EINVAL);
                            }

                            match req.value {
                                0 | 1 => {
                                    self.writeback_connectors
                                        .store(req.value == 1, Ordering::Relaxed);
                                }
                                _ => return_errno!(Errno::EINVAL),
                            };
                        }
                        DrmSetCapabilities::CursorPlaneHostport => {
                            if !self.device().check_feature(DrmFeatures::CURSOR_HOTSPOT) {
                                return_errno!(Errno::EOPNOTSUPP);
                            }

                            if !self.atomic.load(Ordering::Relaxed) {
                                return_errno!(Errno::EINVAL);
                            }

                            match req.value {
                                0 | 1 => {
                                    self.supports_virtualized_cursor_plane
                                        .store(req.value == 1, Ordering::Relaxed);
                                }
                                _ => return_errno!(Errno::EINVAL),
                            };
                        }
                    }

                    Ok(0)
                }
                _cmd @ DrmIoctlSetMaster => {
                    // TODO
                    Ok(0)
                }
                _cmd @ DrmIoctlDropMaster => {
                    // TODO
                    Ok(0)
                }
                cmd @ DrmIoctlModeGetResources => {
                    if !self.device().check_feature(DrmFeatures::MODESET) {
                        return_errno!(Errno::EOPNOTSUPP);
                    }

                    let mut card_res: DrmModeGetResources = cmd.read()?;

                    let dev = self.device();
                    let config = dev.mode_config().lock();

                    let count_crtcs = config.count_objects(DrmObjectType::Crtc) as u32;
                    let count_encoders = config.count_objects(DrmObjectType::Encoder) as u32;
                    let count_connectors = config.count_objects(DrmObjectType::Connector) as u32;
                    let count_fbs = config.count_objects(DrmObjectType::Framebuffer) as u32;

                    if card_res.is_first_call() {
                        card_res.count_crtcs = count_crtcs;
                        card_res.count_encoders = count_encoders;
                        card_res.count_connectors = count_connectors;
                        card_res.count_fbs = count_fbs;

                        cmd.write(&card_res)?;
                    } else {
                        if card_res.count_crtcs < count_crtcs
                            || card_res.count_encoders < count_encoders
                            || card_res.count_connectors < count_connectors
                            || card_res.count_fbs < count_fbs
                        {
                            return_errno!(Errno::EFAULT);
                        }

                        write_to_user::<u32>(
                            card_res.crtc_id_ptr as usize,
                            config.collect_object_ids(DrmObjectType::Crtc, None),
                        )?;

                        write_to_user::<u32>(
                            card_res.encoder_id_ptr as usize,
                            config.collect_object_ids(DrmObjectType::Encoder, None),
                        )?;

                        write_to_user::<u32>(
                            card_res.connector_id_ptr as usize,
                            config.collect_object_ids(DrmObjectType::Connector, None),
                        )?;

                        write_to_user::<u32>(
                            card_res.fb_id_ptr as usize,
                            config.collect_object_ids(DrmObjectType::Framebuffer, None),
                        )?;

                        card_res.max_width = config.max_width();
                        card_res.max_height = config.max_height();
                        card_res.min_width = config.min_width();
                        card_res.min_height = config.min_height();

                        cmd.write(&card_res)?;
                    }

                    Ok(0)
                }
                cmd @ DrmIoctlModeGetCrtc => {
                    if !self.device().check_feature(DrmFeatures::MODESET) {
                        return_errno!(Errno::EOPNOTSUPP);
                    }

                    let mut crtc_resp: DrmModeCrtc = cmd.read()?;
                    let dev = self.device();
                    let config = dev.mode_config().lock();

                    if let Some(crtc) = config.get_object_with::<dyn DrmCrtc>(crtc_resp.crtc_id) {
                        let primary = crtc.primary_plane();

                        crtc_resp.gamma_size = crtc.gamma_size();
                        crtc_resp.x = primary.src_x() >> 16;
                        crtc_resp.y = primary.src_y() >> 16;
                        crtc_resp.fb_id = primary.fb_id().unwrap_or(0);

                        if crtc.enable() {
                            if let Some(mode) = crtc.display_mode() {
                                crtc_resp.mode = mode.into();
                            }
                            crtc_resp.mode_valid = 1;
                        } else {
                            crtc_resp.mode_valid = 0;
                        }

                        cmd.write(&crtc_resp)?;
                    } else {
                        return_errno!(Errno::ENOENT);
                    }

                    // TODO:
                    //	if (!file_priv->aspect_ratio_allowed)
                    // crtc_resp.mode.flags &= ~DRM_MODE_FLAG_PIC_AR_MASK;

                    Ok(0)
                }
                cmd @ DrmIoctlModeSetCrtc => {
                    log::error!("[kernel] DrmIoctlModeSetCrtc");
                    if !self.device().check_feature(DrmFeatures::MODESET) {
                        return_errno!(Errno::EOPNOTSUPP);
                    }

                    let crtc_resp: DrmModeCrtc = cmd.read()?;
                    let connector_ids = read_from_user::<u32>(
                        crtc_resp.set_connectors_ptr as usize,
                        crtc_resp.count_connectors as usize,
                    )?;

                    self.device().set_config(&crtc_resp, connector_ids)?;

                    cmd.write(&crtc_resp)?;

                    Ok(0)
                }
                cmd @ DrmIoctlModeCursor => {
                    let _req: DrmModeCursor = cmd.read()?;

                    // TODO
                    return_errno!(Errno::ENXIO);
                    Ok(0)
                }
                cmd @ DrmIoctlModeGetEncoder => {
                    if !self.device().check_feature(DrmFeatures::MODESET) {
                        return_errno!(Errno::EOPNOTSUPP);
                    }

                    let mut encoder_resp: DrmModeGetEncoder = cmd.read()?;
                    let dev = self.device();
                    let config = dev.mode_config().lock();

                    if let Some(encoder) =
                        config.get_object_with::<dyn DrmEncoder>(encoder_resp.encoder_id)
                    {
                        // TODO:
                        encoder_resp.crtc_id = encoder.crtc_id().unwrap_or(0);
                        encoder_resp.encoder_type = encoder.type_() as u32;
                        encoder_resp.possible_crtcs = encoder.possible_crtcs();
                        encoder_resp.possible_clones = encoder.possible_clones();

                        cmd.write(&encoder_resp)?;
                    } else {
                        return_errno!(Errno::ENOENT);
                    }

                    Ok(0)
                }
                cmd @ DrmIoctlModeGetConnector => {
                    if !self.device().check_feature(DrmFeatures::MODESET) {
                        return_errno!(Errno::EOPNOTSUPP);
                    }

                    let mut out_resp: DrmModeGetConnector = cmd.read()?;
                    let dev = self.device();
                    let config = dev.mode_config().lock();

                    if let Some(connector) =
                        config.get_object_with::<dyn DrmConnector>(out_resp.connector_id)
                    {
                        if out_resp.is_first_call() {
                            // get modes
                            // TODO: is current master
                            if true {
                                connector.fill_modes(self.device())?;
                            }

                            out_resp.count_encoders = connector.count_encoders();
                            out_resp.count_modes = connector.count_modes();
                            out_resp.count_props = connector.count_props();

                            cmd.write(&out_resp)?;
                        } else {
                            if out_resp.count_encoders < connector.count_encoders()
                                || out_resp.count_modes < connector.count_modes()
                                || out_resp.count_props < connector.count_props()
                            {
                                return_errno!(Errno::EFAULT);
                            }

                            write_to_user::<u32>(
                                out_resp.encoders_ptr as usize,
                                config.collect_object_ids(
                                    DrmObjectType::Encoder,
                                    Some(connector.possible_encoders()),
                                ),
                            )?;

                            let mode_info = connector.modes().into_iter().map(Into::into);
                            write_to_user::<DrmModeModeInfo>(
                                out_resp.modes_ptr as usize,
                                mode_info.collect(),
                            )?;

                            let properties = connector.get_properties();
                            write_to_user::<u32>(
                                out_resp.props_ptr as usize,
                                properties.keys().copied().collect(),
                            )?;
                            write_to_user::<u64>(
                                out_resp.prop_values_ptr as usize,
                                properties.values().copied().collect(),
                            )?;

                            out_resp.encoder_id = connector.encoder_id().unwrap_or(0);

                            out_resp.connector_type = connector.type_() as u32;
                            out_resp.connector_type_id = connector.type_id_();
                            out_resp.connection = connector.status() as u32;

                            out_resp.mm_width = connector.mm_width();
                            out_resp.mm_height = connector.mm_height();
                            out_resp.subpixel = connector.subpixel();

                            out_resp.pad = 0;
                            cmd.write(&out_resp)?;
                        }
                    } else {
                        return_errno!(Errno::ENOENT);
                    }

                    Ok(0)
                }
                cmd @ DrmIoctlModeGetProperty => {
                    if !self.device().check_feature(DrmFeatures::MODESET) {
                        return_errno!(Errno::EOPNOTSUPP);
                    }

                    let mut out_resp: DrmModeGetProperty = cmd.read()?;
                    let dev = self.device();
                    let config = dev.mode_config().lock();

                    if let Some(property) = config.get_object_with::<DrmProperty>(out_resp.prop_id)
                    {
                        if out_resp.is_first_call() {
                            out_resp.count_values = property.count_values();
                            out_resp.count_enum_blobs = property.count_enum_blobs();
                            out_resp.flags = property.flags();
                            out_resp.name = property.name_to_u8();

                            cmd.write(&out_resp)?;
                        } else {
                            if out_resp.count_values < property.count_values()
                                || out_resp.count_enum_blobs < property.count_enum_blobs()
                            {
                                return_errno!(Errno::EFAULT);
                            }

                            match property.kind() {
                                DrmPropertyKind::Range { min, max } => {
                                    write_to_user::<u64>(
                                        out_resp.values_ptr as usize,
                                        [*min, *max].to_vec(),
                                    )?;
                                }
                                DrmPropertyKind::SignedRange { min, max } => {
                                    write_to_user::<i64>(
                                        out_resp.values_ptr as usize,
                                        [*min, *max].to_vec(),
                                    )?;
                                }
                                DrmPropertyKind::Enum(items) | DrmPropertyKind::Bitmask(items) => {
                                    write_to_user::<PropertyEnum>(
                                        out_resp.enum_blob_ptr as usize,
                                        items.to_vec(),
                                    )?;
                                }
                                _ => {}
                            }

                            out_resp.name = property.name_to_u8();

                            cmd.write(&out_resp)?;
                        }
                    } else {
                        return_errno!(Errno::ENOENT);
                    }

                    Ok(0)
                }
                cmd @ DrmIoctlModeGetPropBlob => {
                    if !self.device().check_feature(DrmFeatures::MODESET) {
                        return_errno!(Errno::EOPNOTSUPP);
                    }

                    let mut out_resp: DrmModeGetBlob = cmd.read()?;
                    let dev = self.device();
                    let config = dev.mode_config().lock();

                    if let Some(blob) = config.get_object_with::<DrmModeBlob>(out_resp.blob_id) {
                        if out_resp.is_first_call() {
                            out_resp.length = blob.length() as u32;

                            cmd.write(&out_resp)?;
                        } else {
                            write_to_user(out_resp.data as usize, blob.data())?;

                            cmd.write(&out_resp)?;
                        }
                    } else {
                        return_errno!(Errno::ENOENT);
                    }

                    Ok(0)
                }
                cmd @ DrmIoctlModeAddFB => {
                    log::error!("[kernel] DrmIoctlModeAddFB");
                    if !self.device().check_feature(DrmFeatures::MODESET) {
                        return_errno!(Errno::EOPNOTSUPP);
                    }

                    let mut fb_cmd: DrmModeFbCmd = cmd.read()?;
                    let dev = self.device();

                    if let Some(gem_object) = self.lookup_gem(fb_cmd.handle) {
                        let fb_cmd2: DrmModeFbCmd2 = fb_cmd.into();
                        let fb_id = dev.fb_create(&fb_cmd2, gem_object)?;
                        fb_cmd.fb_id = fb_id;
                    } else {
                        return_errno!(Errno::ENOENT);
                    }

                    cmd.write(&fb_cmd)?;

                    Ok(0)
                }
                cmd @ DrmIoctlModeRmFB => {
                    if !self.device().check_feature(DrmFeatures::MODESET) {
                        return_errno!(Errno::EOPNOTSUPP);
                    }

                    let fb_id: u32 = cmd.read()?;
                    let dev = self.device();
                    let mut config = dev.mode_config().lock();

                    if let Some(_) = config.remove_object_with::<dyn DrmFramebuffer>(fb_id) {
                    } else {
                        return_errno!(Errno::ENOENT);
                    }

                    Ok(0)
                }
                cmd @ DrmIoctlModePageFlip => {
                    if !self.device().check_feature(DrmFeatures::MODESET) {
                        return_errno!(Errno::EOPNOTSUPP);
                    }

                    let page_flip: DrmModeCrtcPageFlip = cmd.read()?;
                    let flags = PageFlipFlags::from_bits(page_flip.flags).ok_or(Errno::EINVAL)?;
                    let dev = self.device();

                    // Only one of the TARGET_ABSOLUTE/TARGET_RELATIVE flags
                    // can be specified
                    if flags.contains(PageFlipFlags::TARGET) {
                        return_errno!(Errno::EINVAL);
                    }

                    if flags.intersects(PageFlipFlags::ASYNC)
                        && !(dev.mode_config().lock().allow_async_page_flip())
                    {
                        return_errno!(Errno::EINVAL);
                    }

                    // let sequence = if flags
                    //     .intersects(PageFlipFlags::TARGET_ABSOLUTE | PageFlipFlags::TARGET_RELATIVE)
                    // {
                    //     page_flip.reserved
                    // } else {
                    //     if page_flip.reserved !=0 {
                    //         return_errno!(Errno::EINVAL);
                    //     }
                    //     0
                    // };

                    // let page_flip: DrmModeCrtcPageFlipTarget = DrmModeCrtcPageFlipTarget {
                    //     crtc_id: page_flip.crtc_id,
                    //     fb_id: page_flip.fb_id,
                    //     flags: page_flip.flags,
                    //     sequence,
                    //     user_data: page_flip.user_data,
                    // };

                    let target: Option<u32> = if flags.intersects(PageFlipFlags::TARGET) {
                        // TODO: Vblank target support
                        return_errno!(Errno::EOPNOTSUPP);
                    } else {
                        None
                    };

                    dev.page_flip(&page_flip, self.vblank.clone(), target)?;

                    Ok(0)
                }
                cmd @ DrmIoctlModeDirtyFb => {
                    if !self.device().check_feature(DrmFeatures::MODESET) {
                        return_errno!(Errno::EOPNOTSUPP);
                    }
                    let dirty_cmd: DrmModeFbDirtyCmd = cmd.read()?;
                    // TODO

                    self.device().dirty_framebuffer(dirty_cmd.fb_id)?;

                    Ok(0)
                }
                cmd @ DrmIoctlModeCreateDumb => {
                    log::error!("[kernel] DrmIoctlModeCreateDumb");
                    if !self.device().check_feature(DrmFeatures::MODESET) {
                        return_errno!(Errno::EOPNOTSUPP);
                    }

                    if !self.device().check_capbility(DrmDeviceCaps::DUMB_CREATE) {
                        return_errno!(Errno::ENOSYS);
                    }

                    let mut args: DrmModeCreateDumb = cmd.read()?;
                    let dev = self.device();

                    let gem_object = dev.create_dumb(&mut args, memfd_allocator)?;
                    let handle = self.next_gem_handle();
                    args.pitch = gem_object.pitch();
                    args.size = gem_object.size();
                    args.handle = handle;

                    self.add_gem(handle, gem_object);

                    cmd.write(&args)?;

                    Ok(0)
                }
                cmd @ DrmIoctlModeMapDumb => {
                    log::error!("[kernel] DrmIoctlModeMapDumb");
                    if !self.device().check_feature(DrmFeatures::MODESET) {
                        return_errno!(Errno::EOPNOTSUPP);
                    }

                    if !self.device().check_capbility(DrmDeviceCaps::DUMB_CREATE) {
                        return_errno!(Errno::ENOSYS);
                    }

                    let mut args: DrmModeMapDumb = cmd.read()?;
                    let dev = self.device();

                    args.offset = dev.map_dumb(args.handle)?;

                    cmd.write(&args)?;
                    Ok(0)
                }
                cmd @ DrmIoctlModeDestroyDumb => {
                    log::error!("[kernel] DrmIoctlModeDestroyDumb");
                    if !self.device().check_feature(DrmFeatures::MODESET) {
                        return_errno!(Errno::EOPNOTSUPP);
                    }

                    if !self.device().check_capbility(DrmDeviceCaps::DUMB_CREATE) {
                        return_errno!(Errno::ENOSYS);
                    }

                    let args: DrmModeDestroyDumb = cmd.read()?;

                    if let Some(gem_object) = self.remove_gem(args.handle) {
                        gem_object.release()?;
                    } else {
                        return_errno!(Errno::ENOENT);
                    }

                    Ok(0)
                }
                cmd @ DrmIoctlModeGetPlaneResources => {
                    if !self.device().check_feature(DrmFeatures::MODESET) {
                        return_errno!(Errno::EOPNOTSUPP);
                    }

                    let mut plane_resp: DrmModeGetPlaneRes = cmd.read()?;
                    let dev = self.device();
                    let config = dev.mode_config().lock();

                    let count_planes = config.count_objects(DrmObjectType::Plane) as u32;

                    if plane_resp.is_first_call() {
                        plane_resp.count_planes = count_planes;

                        cmd.write(&plane_resp)?;
                    } else {
                        if plane_resp.count_planes < count_planes {
                            return_errno!(Errno::EFAULT);
                        }
                        write_to_user(
                            plane_resp.plane_id_ptr as usize,
                            config.collect_object_ids(DrmObjectType::Plane, None),
                        )?;

                        cmd.write(&plane_resp)?;
                    }

                    Ok(0)
                }
                cmd @ DrmIoctlModeGetPlane => {
                    if !self.device().check_feature(DrmFeatures::MODESET) {
                        return_errno!(Errno::EOPNOTSUPP);
                    }

                    let mut plane_resp: DrmModeGetPlane = cmd.read()?;
                    let dev = self.device();
                    let config = dev.mode_config().lock();

                    if let Some(plane) = config.get_object_with::<dyn DrmPlane>(plane_resp.plane_id)
                    {
                        // TODO:
                        plane_resp.gamma_size = 0;
                        plane_resp.possible_crtcs = plane.possible_crtcs();
                        plane_resp.crtc_id = plane.crtc_id().unwrap_or(0);
                        plane_resp.fb_id = plane.fb_id().unwrap_or(0);

                        cmd.write(&plane_resp)?;
                    } else {
                        return_errno!(Errno::ENOENT);
                    }

                    Ok(0)
                }
                cmd @ DrmIoctlModeAddFB2 => {
                    log::error!("[kernel] DrmIoctlModeAddFB2");
                    if !self.device().check_feature(DrmFeatures::MODESET) {
                        return_errno!(Errno::EOPNOTSUPP);
                    }
                    let mut fb_cmd: DrmModeFbCmd2 = cmd.read()?;

                    let dev = self.device();

                    {
                        let flags = DrmModeFb::from_bits(fb_cmd.flags).ok_or(Errno::EINVAL)?;
                        let config = dev.mode_config().lock();

                        if fb_cmd.width < config.min_width()
                            || fb_cmd.width > config.max_width()
                            || fb_cmd.height < config.min_height()
                            || fb_cmd.height > config.max_height()
                        {
                            return_errno!(Errno::EINVAL);
                        }

                        if flags.contains(DrmModeFb::MODIFIERS)
                            && config.fb_modifiers_not_supported()
                        {
                            return_errno!(Errno::EINVAL);
                        }
                    }

                    if let Some(gem_object) = self.lookup_gem(fb_cmd.handles[0]) {
                        let fb_id = dev.fb_create(&fb_cmd, gem_object)?;
                        fb_cmd.fb_id = fb_id;
                    } else {
                        return_errno!(Errno::ENOENT);
                    }

                    cmd.write(&fb_cmd)?;

                    Ok(0)
                }
                cmd @ DrmIoctlModeObjectGetProps => {
                    if !self.device().check_feature(DrmFeatures::MODESET) {
                        return_errno!(Errno::EOPNOTSUPP);
                    }

                    let mut arg: DrmModeObjectGetProps = cmd.read()?;
                    let dev = self.device();
                    let config = dev.mode_config().lock();

                    if let Some(object) = config.get_object(arg.obj_id, arg.obj_type.try_into()?) {
                        if arg.is_first_call() {
                            arg.count_props = object.count_props();
                            cmd.write(&arg)?;
                        } else {
                            if arg.count_props < object.count_props() {
                                return_errno!(Errno::EFAULT);
                            }

                            let properties = object.get_properties();
                            write_to_user::<u32>(
                                arg.props_ptr as usize,
                                properties.keys().copied().collect(),
                            )?;
                            write_to_user::<u64>(
                                arg.prop_values_ptr as usize,
                                properties.values().copied().collect(),
                            )?;
                        }
                    } else {
                        return_errno!(Errno::ENOENT);
                    }

                    Ok(0)
                }
                cmd @ DrmIoctlModeCursor2 => {
                    let _req: DrmModeCursor2 = cmd.read()?;

                    return_errno!(Errno::ENXIO);
                    Ok(0)
                }
                cmd @ DrmIoctlModeAtomic => {
                    if !self.device().check_feature(DrmFeatures::ATOMIC)
                        || !self.device().check_feature(DrmFeatures::MODESET)
                        || !self.atomic.load(Ordering::Relaxed)
                    {
                        return_errno!(Errno::EOPNOTSUPP);
                    }

                    let arg: DrmModeAtomic = cmd.read()?;
                    let flags = DrmAtomicFlags::from_bits(arg.flags).ok_or(Errno::EINVAL)?;
                    if arg.reserved != 0 {
                        return_errno!(Errno::EINVAL);
                    }

                    // get user space data
                    let object_ids =
                        read_from_user::<u32>(arg.objs_ptr as usize, arg.count_objs as usize)?;
                    let prop_counts = read_from_user::<u32>(
                        arg.count_props_ptr as usize,
                        arg.count_objs as usize,
                    )?;
                    let total_props = prop_counts
                        .iter()
                        .try_fold(0usize, |acc, c| acc.checked_add(*c as usize))
                        .ok_or(Errno::EINVAL)?;

                    let prop_ids = read_from_user::<u32>(arg.props_ptr as usize, total_props)?;
                    let prop_values =
                        read_from_user::<u64>(arg.prop_values_ptr as usize, total_props)?;

                    // Atomic check
                    let mut atomic_states = DrmAtomicPendingState::new();
                    let requires_modeset = atomic_states.init(
                        self.device(),
                        object_ids,
                        prop_counts,
                        prop_ids,
                        prop_values,
                    )?;

                    // Basic Linux-like modeset gating.
                    if requires_modeset && !flags.contains(DrmAtomicFlags::ALLOW_MODESET) {
                        return_errno!(Errno::EINVAL);
                    }

                    // Only check
                    if flags.contains(DrmAtomicFlags::TEST_ONLY) {
                        return Ok(0);
                    }

                    let nonblock = flags.contains(DrmAtomicFlags::NONBLOCK);
                    let page_flip_event = if flags.contains(DrmAtomicFlags::PAGE_FLIP_EVENT) {
                        Some(PageFlipEvent::new(arg.user_data, self.vblank.clone()))
                    } else {
                        None
                    };

                    self.device()
                        .atomic_commit(nonblock, &mut atomic_states, page_flip_event)?;

                    Ok(0)
                }
                cmd @ DrmIoctlModeCreatePropBlob => {
                    if !self.device().check_feature(DrmFeatures::MODESET) {
                        return_errno!(Errno::EOPNOTSUPP);
                    }

                    let mut out_resp: DrmModeCreateBlob = cmd.read()?;
                    let dev = self.device();
                    let mut config = dev.mode_config().lock();

                    let data =
                        read_from_user::<u8>(out_resp.data as usize, out_resp.length as usize)?;

                    out_resp.blob_id = config.add_blob(data);

                    cmd.write(&out_resp)?;
                    Ok(0)
                }
                cmd @ DrmIoctlModeDestroyPropBlob => {
                    if !self.device().check_feature(DrmFeatures::MODESET) {
                        return_errno!(Errno::EOPNOTSUPP);
                    }

                    let out_resp: DrmModeDestroyBlob = cmd.read()?;
                    let dev = self.device();
                    let mut config = dev.mode_config().lock();

                    if let Some(_) = config.remove_object_with::<DrmModeBlob>(out_resp.blob_id) {
                    } else {
                        return_errno!(Errno::ENOENT);
                    }

                    Ok(0)
                }
                _ => {
                    log::error!("[kernel] the ioctl {:?} command is unknown", raw_ioctl);
                    return_errno_with_message!(Errno::ENOTTY, "the ioctl command is unknown");
                }
            }
        )
    }
}

fn read_from_user<T: Pod>(ptr: usize, count: usize) -> Result<Vec<T>> {
    if count == 0 {
        return Ok(Vec::new());
    }
    if ptr == 0 {
        return_errno!(Errno::EFAULT);
    }

    let mut values = Vec::with_capacity(count);
    for i in 0..count {
        let offset = ptr + i * core::mem::size_of::<T>();
        values.push(current_userspace!().read_val(offset)?);
    }
    Ok(values)
}

fn write_to_user<T: Pod>(ptr: usize, values: Vec<T>) -> Result<()> {
    if values.len() == 0 {
        return Ok(());
    }

    if ptr == 0 {
        return_errno!(Errno::EFAULT);
    }

    for (i, val) in values.iter().enumerate() {
        let offset = ptr + i * core::mem::size_of::<T>();
        current_userspace!().write_val(offset, val)?;
    }
    Ok(())
}
