use alloc::sync::Arc;

use aster_gpu::drm::{
    DrmDevice, DrmFeatures,
    drm_modes::DrmModeModeInfo,
    ioctl::*,
    mode_object::{
        DrmObjectType,
        connector::DrmConnector,
        crtc::DrmCrtc,
        encoder::DrmEncoder,
        property::{DrmModeBlob, DrmProperty, PropertyEnum, PropertyKind},
    },
};
use ostd::mm::VmIo;

use crate::{
    current_userspace,
    device::drm::{ioctl_defs::*, minor::DrmMinor},
    dispatch_drm_ioctl,
    events::IoEvents,
    fs::{
        file_handle::Mappable,
        inode_handle::FileIo,
        utils::{InodeIo, StatusFlags},
    },
    prelude::*,
    process::signal::{PollHandle, Pollable},
    util::ioctl::RawIoctl,
};

#[derive(Debug)]
pub(super) struct DrmFile {
    minor: Arc<DrmMinor>,
}

impl DrmFile {
    pub fn new(minor: Arc<DrmMinor>) -> Self {
        Self { minor }
    }

    pub fn device(&self) -> Arc<dyn DrmDevice> {
        self.minor.device()
    }
}

impl Pollable for DrmFile {
    fn poll(&self, mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        let events = IoEvents::IN | IoEvents::OUT;
        events & mask
    }
}

impl InodeIo for DrmFile {
    fn read_at(
        &self,
        _offset: usize,
        _writer: &mut VmWriter,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "drm: read not supported");
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

    fn mappable(&self) -> Result<Mappable> {
        return_errno!(Errno::EINVAL);
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
                    }

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
                            config.get_object_ids(DrmObjectType::Crtc, None),
                        )?;

                        write_to_user::<u32>(
                            card_res.encoder_id_ptr as usize,
                            config.get_object_ids(DrmObjectType::Encoder, None),
                        )?;

                        write_to_user::<u32>(
                            card_res.connector_id_ptr as usize,
                            config.get_object_ids(DrmObjectType::Connector, None),
                        )?;

                        write_to_user::<u32>(
                            card_res.fb_id_ptr as usize,
                            config.get_object_ids(DrmObjectType::Framebuffer, None),
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
                        crtc_resp.fb_id = primary.fb_id();

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
                        encoder_resp.crtc_id = 0;
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
                                connector.fill_modes()?;
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
                                config.get_object_ids(
                                    DrmObjectType::Encoder,
                                    Some(connector.possible_encoders()),
                                ),
                            )?;

                            write_to_user::<DrmModeModeInfo>(
                                out_resp.modes_ptr as usize,
                                connector.modes().into_iter().map(Into::into),
                            )?;

                            let properties = connector.get_properties();
                            write_to_user::<u32>(
                                out_resp.props_ptr as usize,
                                properties.keys().copied(),
                            )?;
                            write_to_user::<u64>(
                                out_resp.prop_values_ptr as usize,
                                properties.values().copied(),
                            )?;

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

                            cmd.write(&out_resp)?;
                        } else {
                            if out_resp.count_values < property.count_values()
                                || out_resp.count_enum_blobs < property.count_enum_blobs()
                            {
                                return_errno!(Errno::EFAULT);
                            }

                            match property.kind() {
                                PropertyKind::Range { min, max } => {
                                    write_to_user::<u64>(
                                        out_resp.values_ptr as usize,
                                        [*min, *max],
                                    )?;
                                }
                                PropertyKind::SignedRange { min, max } => {
                                    write_to_user::<i64>(
                                        out_resp.values_ptr as usize,
                                        [*min, *max],
                                    )?;
                                }
                                PropertyKind::Enum(items) | PropertyKind::Bitmask(items) => {
                                    write_to_user::<PropertyEnum>(
                                        out_resp.enum_blob_ptr as usize,
                                        items.into_iter().copied(),
                                    )?;
                                }
                                _ => {}
                            }

                            out_resp.name = property.name();

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
                                properties.keys().copied(),
                            )?;
                            write_to_user::<u64>(
                                arg.prop_values_ptr as usize,
                                properties.values().copied(),
                            )?;
                        }
                    } else {
                        return_errno!(Errno::ENOENT);
                    }

                    Ok(0)
                }
                cmd @ DrmIoctlModeCreatePropBlob => {
                    if !self.device().check_feature(DrmFeatures::MODESET) {
                        return_errno!(Errno::EOPNOTSUPP);
                    }

                    let mut out_resp: DrmModeCreateBlob = cmd.read()?;
                    let dev = self.device();
                    let mut config = dev.mode_config().lock();

                    let mut data = vec![0u8; out_resp.length as usize];
                    for (i, item) in data.iter_mut().enumerate() {
                        let offset = out_resp.data as usize + i * size_of::<u8>();
                        *item = current_userspace!().read_val(offset)?;
                    }

                    out_resp.blob_id = config.add_blob(data) as u32;

                    cmd.write(&out_resp)?;
                    Ok(0)
                }
                _ => {
                    return_errno_with_message!(Errno::ENOTTY, "the ioctl command is unknown");
                }
            }
        )
    }
}

fn write_to_user<T: Pod>(ptr: usize, values: impl IntoIterator<Item = T>) -> Result<()> {
    for (i, val) in values.into_iter().enumerate() {
        let offset = ptr + i * core::mem::size_of::<T>();
        current_userspace!().write_val(offset, &val)?;
    }
    Ok(())
}
