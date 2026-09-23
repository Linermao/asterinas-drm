// SPDX-License-Identifier: MPL-2.0

//! A simpledrm driver backed by the bootloader-provided framebuffer.
//!
//! It obtains framebuffer information from `aster-framebuffer` and registers
//! the resulting DRM device with `aster-drm`.

#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

// Set this crate's log prefix for `ostd::log`.
macro_rules! __log_prefix {
    () => {
        "simpledrm: "
    };
}

use alloc::{sync::Arc, vec, vec::Vec};
use core::fmt::Debug;

use aster_core::prelude::*;
use aster_drm::{
    device::{DrmDevice, DrmFeatures},
    gem::{DrmGemOps, object::DrmGemObject, shmem::DrmGemShmemObject},
    kms::{
        DrmKmsDevice, DrmModeConfig,
        objects::{
            DrmKmsObjectType, KmsObjectId,
            builder::DrmKmsObjectBuilder,
            connector::{
                DrmConnType, DrmConnectorProbeState, DrmConnectorState, DrmConnectorStatus,
            },
            crtc::DrmCrtcState,
            encoder::DrmEncoderType,
            plane::{DrmPlaneState, DrmPlaneType},
        },
    },
    utils::{DrmDisplayFormat, DrmDisplayInfo, DrmDisplayMode, DrmRect, DrmSize, SubpixelOrder},
};
use aster_framebuffer::{
    framebuffer::{self, FrameBuffer},
    pixel::PixelFormat,
};
use component::{ComponentInitError, init_component};
use ostd::mm::VmWriter;

const SIMPLEDRM_NAME: &str = "simpledrm";
const SIMPLEDRM_DESC: &str = "DRM driver for simple-framebuffer platform devices";

const SIMPLEDRM_VREFRESH_HZ: u32 = 60;
const SIMPLEDRM_ASSUMED_DPI: u32 = 96;

#[init_component(process)]
fn init() -> Result<(), ComponentInitError> {
    let Some(framebuffer) = framebuffer::FRAMEBUFFER.get() else {
        ostd::warn!("Failed to init: boot framebuffer is unavailable");
        return Ok(());
    };

    let device = match SimpleDrmDevice::new(framebuffer) {
        Ok(device) => device,
        Err(err) => {
            ostd::warn!("Failed to create device: {:?}", err);
            return Ok(());
        }
    };

    if let Err(err) = aster_drm::register_device(Arc::new(device)) {
        ostd::warn!("Failed to register device: {:?}", err);
    }

    Ok(())
}

#[derive(Debug)]
struct SimpleDrmDevice {
    boot_framebuffer: Arc<FrameBuffer>,
    features: DrmFeatures,
    format_types: DrmDisplayFormat,
    mode_config: DrmModeConfig,
}

impl SimpleDrmDevice {
    fn new(framebuffer: &Arc<FrameBuffer>) -> Result<Self> {
        let mut builder = DrmKmsObjectBuilder::default();
        let format_types = match framebuffer.pixel_format() {
            PixelFormat::BgrReserved => DrmDisplayFormat::XRGB8888,
            format => {
                // TODO: Derive the exact DRM format once framebuffer initialization
                // preserves the complete boot framebuffer layout.
                // See: `kernel/core/comps/framebuffer/src/framebuffer.rs:73`.
                //
                // Until then, advertise XRGB8888 so simpledrm remains available for
                // KMS query tests. This may not match the actual framebuffer layout.
                ostd::warn!(
                    "Boot framebuffer format {:?} has no DRM mapping; assuming XRGB8888",
                    format
                );
                DrmDisplayFormat::XRGB8888
            }
        };
        let primary = builder.add_plane(DrmPlaneType::Primary, vec![format_types]);
        let crtc = builder.add_crtc(0, primary, None);
        let encoder = builder.add_encoder(DrmEncoderType::VIRTUAL);
        let connector = builder.add_connector(DrmConnType::VIRTUAL);

        builder.plane_attach_crtc(primary, crtc)?;
        builder.encoder_attach_crtc(encoder, crtc)?;
        builder.connector_attach_encoder(connector, encoder)?;

        let object_store = builder.build()?;

        let width = u32::try_from(framebuffer.width())?;
        let height = u32::try_from(framebuffer.height())?;

        let mode_config = DrmModeConfig::new(
            DrmSize::new(1, 1),
            DrmSize::new(width, height),
            object_store,
        )?
        .with_shadow_buffer();

        Ok(Self {
            boot_framebuffer: framebuffer.clone(),
            features: DrmFeatures::MODESET | DrmFeatures::GEM,
            format_types,
            mode_config,
        })
    }

    fn flush_framebuffer(&self, fb_id: KmsObjectId, source_rect: DrmRect) -> Result<()> {
        let (width, height, pitch, pixel_format, gem_object) = {
            let object_store = self.mode_config.object_store().lock();
            let framebuffer = object_store
                .lookup_framebuffer(fb_id)
                .ok_or(Errno::ENOENT)?;
            let gem_object = framebuffer.gem_object().cloned().ok_or(Errno::EINVAL)?;

            (
                framebuffer.width(),
                framebuffer.height(),
                framebuffer.pitches(),
                framebuffer.pixel_format(),
                gem_object,
            )
        };

        if pixel_format != self.format_types {
            return_errno_with_message!(
                Errno::EINVAL,
                "the DRM framebuffer format is not supported by simpledrm"
            );
        }

        let framebuffer_rect = DrmRect::new(0, 0, width, height);
        if !framebuffer_rect.contains_rect(&source_rect) {
            return_errno_with_message!(
                Errno::EINVAL,
                "the simpledrm scanout rectangle exceeds the DRM framebuffer"
            );
        }

        let output_width = u32::try_from(self.boot_framebuffer.width())?;
        let output_height = u32::try_from(self.boot_framebuffer.height())?;
        if source_rect.width() != output_width || source_rect.height() != output_height {
            return_errno_with_message!(
                Errno::EINVAL,
                "the simpledrm scanout size does not match the physical framebuffer"
            );
        }

        let bytes_per_pixel = pixel_format.bytes_per_pixel();
        let row_len = self
            .boot_framebuffer
            .width()
            .checked_mul(bytes_per_pixel)
            .ok_or(Errno::EOVERFLOW)?;
        if row_len > self.boot_framebuffer.line_size() {
            return_errno_with_message!(
                Errno::EINVAL,
                "the simpledrm scanout row exceeds the physical framebuffer pitch"
            );
        }

        let pitch = usize::try_from(pitch)?;
        let source_x = usize::try_from(source_rect.x())?;
        let source_y = usize::try_from(source_rect.y())?;
        let source_x_bytes = source_x
            .checked_mul(bytes_per_pixel)
            .ok_or(Errno::EOVERFLOW)?;
        let mut row = Vec::new();
        row.try_reserve_exact(row_len)
            .map_err(|_| Error::with_message(Errno::ENOMEM, "failed to allocate a scanout row"))?;
        row.resize(row_len, 0);

        for row_index in 0..self.boot_framebuffer.height() {
            let source_row = source_y.checked_add(row_index).ok_or(Errno::EOVERFLOW)?;
            let source_offset = source_row
                .checked_mul(pitch)
                .and_then(|offset| offset.checked_add(source_x_bytes))
                .ok_or(Errno::EOVERFLOW)?;
            let mut writer = VmWriter::from(row.as_mut_slice()).to_fallible();
            gem_object.read(source_offset, &mut writer)?;

            let destination_offset = row_index
                .checked_mul(self.boot_framebuffer.line_size())
                .ok_or(Errno::EOVERFLOW)?;
            self.boot_framebuffer
                .write_bytes_at(destination_offset, row.as_slice())?;
        }

        Ok(())
    }
}

impl DrmDevice for SimpleDrmDevice {
    fn name(&self) -> &str {
        SIMPLEDRM_NAME
    }

    fn desc(&self) -> &str {
        SIMPLEDRM_DESC
    }

    fn features(&self) -> &DrmFeatures {
        &self.features
    }

    fn kms_device(&self) -> Option<&dyn DrmKmsDevice> {
        Some(self)
    }

    fn gem_ops(&self) -> Option<&dyn DrmGemOps> {
        Some(self)
    }
}

impl DrmKmsDevice for SimpleDrmDevice {
    fn mode_config(&self) -> &DrmModeConfig {
        &self.mode_config
    }

    fn probe_connector(&self, connector_id: KmsObjectId) -> Result<()> {
        let width = u32::try_from(self.boot_framebuffer.width())?;
        let height = u32::try_from(self.boot_framebuffer.height())?;
        let resolution = DrmSize::new(width, height);

        let display_mode = DrmDisplayMode::from_size(resolution, SIMPLEDRM_VREFRESH_HZ)?;
        // `simpledrm` only has the boot framebuffer's pixel geometry here, so
        // it relies on the shared physical-size fallback path.
        let display_info =
            DrmDisplayInfo::from_dpi(resolution, SIMPLEDRM_ASSUMED_DPI, SubpixelOrder::Unknown)?;
        let probe_state = DrmConnectorProbeState::new(
            DrmConnectorStatus::Connected,
            vec![display_mode],
            display_info,
        );

        let object_store = self.mode_config.object_store().lock();
        let connector = object_store
            .lookup_connector(connector_id)
            .ok_or(Errno::ENOENT)?;

        connector.update_probe_state(probe_state);

        Ok(())
    }

    fn set_crtc(
        &self,
        crtc_id: KmsObjectId,
        fb_id: KmsObjectId,
        x: u32,
        y: u32,
        display_mode: Option<DrmDisplayMode>,
        connector_ids: Vec<KmsObjectId>,
    ) -> Result<()> {
        let Some(display_mode) = display_mode else {
            let object_store = self.mode_config.object_store().lock();
            let crtc = object_store.lookup_crtc(crtc_id).ok_or(Errno::ENOENT)?;
            let primary_plane = object_store
                .lookup_plane(crtc.primary_plane_id())
                .ok_or(Errno::ENOENT)?;

            crtc.update_state(DrmCrtcState::default());
            primary_plane.update_state(DrmPlaneState::default());

            for encoder_id in object_store.collect_object_ids(DrmKmsObjectType::Encoder) {
                let encoder = object_store
                    .lookup_encoder(encoder_id)
                    .ok_or(Errno::ENOENT)?;
                if encoder.current_crtc_id() == Some(crtc_id) {
                    encoder.set_current_crtc_id(None);
                }
            }
            for connector_id in object_store.collect_object_ids(DrmKmsObjectType::Connector) {
                let connector = object_store
                    .lookup_connector(connector_id)
                    .ok_or(Errno::ENOENT)?;
                let encoder_id = connector.state_snapshot().encoder_id();
                if encoder_id.is_some_and(|encoder_id| {
                    object_store
                        .lookup_encoder(encoder_id)
                        .is_some_and(|encoder| encoder.current_crtc_id().is_none())
                }) {
                    connector.update_state(DrmConnectorState::default());
                }
            }

            return Ok(());
        };

        if connector_ids.len() != 1 {
            return_errno_with_message!(
                Errno::EINVAL,
                "simpledrm requires exactly one connector for an enabled CRTC"
            );
        }

        let mode_width = u32::from(display_mode.hdisplay());
        let mode_height = u32::from(display_mode.vdisplay());
        if mode_width != u32::try_from(self.boot_framebuffer.width())?
            || mode_height != u32::try_from(self.boot_framebuffer.height())?
        {
            return_errno_with_message!(
                Errno::EINVAL,
                "simpledrm only supports the physical framebuffer display mode"
            );
        }

        let source_rect = DrmRect::new(x, y, mode_width, mode_height);
        let crtc_rect = DrmRect::new(0, 0, mode_width, mode_height);
        let connector_id = connector_ids[0];
        let (primary_plane_id, encoder_id) = {
            let object_store = self.mode_config.object_store().lock();
            let crtc = object_store.lookup_crtc(crtc_id).ok_or(Errno::ENOENT)?;
            let primary_plane = object_store
                .lookup_plane(crtc.primary_plane_id())
                .ok_or(Errno::ENOENT)?;
            let framebuffer = object_store
                .lookup_framebuffer(fb_id)
                .ok_or(Errno::ENOENT)?;
            if !primary_plane
                .format_types()
                .contains(&framebuffer.pixel_format())
            {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "the DRM framebuffer format is unsupported by the primary plane"
                );
            }
            if !DrmRect::new(0, 0, framebuffer.width(), framebuffer.height())
                .contains_rect(&source_rect)
            {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "the simpledrm scanout rectangle exceeds the DRM framebuffer"
                );
            }

            let crtc_index = object_store
                .get_object_index(crtc_id, DrmKmsObjectType::Crtc)
                .ok_or(Errno::ENOENT)?;
            let connector = object_store
                .lookup_connector(connector_id)
                .ok_or(Errno::ENOENT)?;
            let encoder_id = connector
                .possible_encoders()
                .iter()
                .find_map(|encoder_index| {
                    let encoder_id = object_store
                        .get_object_id_from_index(*encoder_index, DrmKmsObjectType::Encoder)?;
                    object_store
                        .lookup_encoder(encoder_id)?
                        .possible_crtcs()
                        .contains(&crtc_index)
                        .then_some(encoder_id)
                })
                .ok_or_else(|| {
                    Error::with_message(
                        Errno::EINVAL,
                        "the connector has no encoder compatible with the CRTC",
                    )
                })?;

            (crtc.primary_plane_id(), encoder_id)
        };

        self.flush_framebuffer(fb_id, source_rect)?;

        let object_store = self.mode_config.object_store().lock();
        let crtc = object_store.lookup_crtc(crtc_id).ok_or(Errno::ENOENT)?;
        let primary_plane = object_store
            .lookup_plane(primary_plane_id)
            .ok_or(Errno::ENOENT)?;
        let encoder = object_store
            .lookup_encoder(encoder_id)
            .ok_or(Errno::ENOENT)?;
        let connector = object_store
            .lookup_connector(connector_id)
            .ok_or(Errno::ENOENT)?;
        if object_store.lookup_framebuffer(fb_id).is_none() {
            return_errno_with_message!(Errno::ENOENT, "the DRM framebuffer was removed");
        }

        crtc.update_state(DrmCrtcState::new(Some(display_mode)));
        primary_plane.update_state(DrmPlaneState::new(
            source_rect,
            crtc_rect,
            Some(fb_id),
            Some(crtc_id),
        ));
        encoder.set_current_crtc_id(Some(crtc_id));
        connector.update_state(DrmConnectorState::new(Some(encoder_id)));

        Ok(())
    }

    fn dirty_fb(&self, fb_id: KmsObjectId) -> Result<()> {
        let source_rect = {
            let object_store = self.mode_config.object_store().lock();
            if object_store.lookup_framebuffer(fb_id).is_none() {
                return_errno_with_message!(Errno::ENOENT, "the DRM framebuffer does not exist");
            }

            object_store
                .collect_object_ids(DrmKmsObjectType::Plane)
                .into_iter()
                .filter_map(|plane_id| object_store.lookup_plane(plane_id))
                .find_map(|plane| {
                    let state = plane.state_snapshot();
                    (state.fb_id() == Some(fb_id)).then(|| state.source_rect())
                })
        };

        if let Some(source_rect) = source_rect {
            self.flush_framebuffer(fb_id, source_rect)?;
        }

        Ok(())
    }
}

impl DrmGemOps for SimpleDrmDevice {
    fn create_dumb(&self, width: u32, height: u32, bpp: u32) -> Result<Arc<dyn DrmGemObject>> {
        let bits_per_row = u64::from(width) * u64::from(bpp);
        let Ok(pitch) = u32::try_from(bits_per_row.div_ceil(8)) else {
            return_errno_with_message!(Errno::EINVAL, "the dumb-buffer pitch overflows");
        };
        let Ok(pitch_size) = usize::try_from(pitch) else {
            return_errno_with_message!(Errno::EINVAL, "the dumb-buffer pitch is too large");
        };
        let Ok(height) = usize::try_from(height) else {
            return_errno_with_message!(Errno::EINVAL, "the dumb-buffer height is too large");
        };
        let Some(size) = pitch_size.checked_mul(height) else {
            return_errno_with_message!(Errno::EINVAL, "the dumb-buffer size overflows");
        };

        Ok(Arc::new(DrmGemShmemObject::new(pitch, size)?))
    }
}
