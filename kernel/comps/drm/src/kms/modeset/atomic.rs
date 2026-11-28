// SPDX-License-Identifier: MPL-2.0

use alloc::{
    collections::BTreeMap,
    sync::{Arc, Weak},
    vec::Vec,
};

use aster_core::prelude::*;

use crate::{
    device::DrmDevice,
    kms::{
        DrmKmsOps,
        display_mode::DrmDisplayMode,
        modeset::{
            KmsModesetState, connector::DrmConnectorState, crtc::DrmCrtcState,
            encoder::DrmEncoderState, plane::DrmPlaneState,
        },
        objects::{
            KmsObjectIndex, KmsObjectStore,
            crtc::DrmCrtc,
            plane::DrmPlaneType,
            property::{DrmStandardProperty, KmsObjectPropValue, blob::DrmPropertyBlob},
        },
    },
    utils::{DrmCrtcRect, DrmRect, DrmSize, DrmU16F16},
};

/// TODO
pub trait DrmAtomicOps: DrmDevice + DrmKmsOps {
    // TODO: add comment
    fn atomic_check(
        &self,
        modeset_state: &KmsModesetState,
        object_store: &KmsObjectStore,
        atomic_state: &mut DrmAtomicState,
    ) -> Result<()>;

    // TODO: add comment
    fn atomic_commit(
        &self,
        modeset_state: &mut KmsModesetState,
        object_store: &KmsObjectStore,
        atomic_state: &DrmAtomicState,
    ) -> Result<()>;

    // TODO: add comment
    fn atomic_commit_tail(&self, modeset_state: &KmsModesetState) -> Result<()>;
}

#[derive(Debug)]
pub(crate) struct DrmAtomicRequest {
    object_id: u32,
    object_props: Vec<(u32, KmsObjectPropValue)>,
}

impl DrmAtomicRequest {
    pub(crate) fn new(object_id: u32, object_props: Vec<(u32, KmsObjectPropValue)>) -> Self {
        Self {
            object_id,
            object_props,
        }
    }
}

#[derive(Debug, Default)]
pub struct DrmAtomicState {
    crtc_states: BTreeMap<KmsObjectIndex, DrmCrtcState>,
    plane_states: BTreeMap<KmsObjectIndex, DrmPlaneState>,
    encoder_states: BTreeMap<KmsObjectIndex, DrmEncoderState>,
    connector_states: BTreeMap<KmsObjectIndex, DrmConnectorState>,
    needs_modeset: bool,
}

impl DrmAtomicState {
    pub(crate) fn needs_modeset(&self) -> bool {
        self.needs_modeset
    }
}

#[derive(Clone, Debug, Default)]
struct PendingCrtcState {
    mode_blob: Option<Arc<DrmPropertyBlob>>,
    is_active: bool,
}

#[derive(Clone, Debug, Default)]
struct PendingConnectorState {
    crtc: Weak<DrmCrtc>,
}

pub fn atomic_helper_check(
    modeset_state: &KmsModesetState,
    object_store: &KmsObjectStore,
    atomic_state: &mut DrmAtomicState,
) -> Result<()> {
    let mut crtcs_with_connectors = 0u32;

    // TODO: add comment
    for connector in object_store.static_objects().connectors() {
        let final_crtc =
            if let Some(pending) = atomic_state.connector_states.get(&connector.index()) {
                pending.crtc().upgrade()
            } else {
                let current = modeset_state.connector_state(connector.index());
                current.crtc().upgrade()
            };

        if let Some(crtc) = final_crtc {
            crtcs_with_connectors |= 1u32 << crtc.index().get();
        }
    }
    for crtc in object_store.static_objects().crtcs() {
        let is_enabled = if let Some(pending_crtc_state) =
            atomic_state.crtc_states.get(&crtc.index())
        {
            let current = modeset_state.crtc_state(crtc.index());

            let mode_changed = match (current.display_mode(), pending_crtc_state.display_mode()) {
                (Some(old), Some(new)) => !old.is_equivalent(&new),
                (None, None) => false,
                _ => true,
            };

            atomic_state.needs_modeset |= mode_changed || current.is_active() != pending_crtc_state.is_active();

            pending_crtc_state.is_enabled()
        } else {
            modeset_state.crtc_state(crtc.index()).is_enabled()
        };

        let has_connectors = crtcs_with_connectors & (1u32 << crtc.index().get()) != 0;

        if is_enabled != has_connectors {
            return Err(errno!(
                EINVAL,
                "the CRTC enable state does not match its connector bindings"
            ));
        }
    }

    // Select encoders for the requested connector states.
    // Preserve the encoder routes of unchanged connectors.
    // Reject encoder conflicts, including conflicts with unchanged connectors.
    // Check connector-to-encoder and encoder-to-CRTC compatibility.
    let mut connector_encoders = BTreeMap::new();
    let mut used_encoders = 0u32;
    for connector in object_store.static_objects().connectors() {
        let encoder = if let Some(pending) = atomic_state.connector_states.get(&connector.index()) {
            let Some(crtc) = pending.crtc().upgrade() else {
                // The connector will be detached.
                continue;
            };

            // TODO: driver-specific selection
            // See <'https://elixir.bootlin.com/linux/v6.17/source/drivers/gpu/drm/drm_atomic_helper.c#L128'>

            // Requires a single possible encoder.
            let mut candidates = connector.possible_encoders().iter_ones();
            let Some(encoder_index) = candidates.next() else {
                return Err(errno!(EINVAL, "the connector has no possible encoder"));
            };

            if candidates.next().is_some() {
                return Err(errno!(
                    EINVAL,
                    "multiple possible encoders require driver encoder selection"
                ));
            }

            let encoder = object_store
                .static_objects()
                .lookup_encoder_by_index(KmsObjectIndex::new(encoder_index))
                .unwrap();

            if !encoder.possible_crtcs()[crtc.index().get()] {
                return Err(errno!(
                    EINVAL,
                    "the encoder cannot be attached to the requested CRTC"
                ));
            }

            encoder.clone()
        } else {
            // Preserve the route of an unchanged connector.
            let current = modeset_state.connector_state(connector.index());
            let Some(encoder) = current.encoder().upgrade() else {
                continue;
            };

            encoder
        };

        let encoder_mask = 1u32 << encoder.index().get();

        if used_encoders & encoder_mask != 0 {
            return Err(errno!(
                EINVAL,
                "an encoder cannot be assigned to multiple connectors"
            ));
        }

        used_encoders |= encoder_mask;
        connector_encoders.insert(connector.index(), encoder);
    }

    for (connector_index, pending) in &mut atomic_state.connector_states {
        let current = modeset_state.connector_state(*connector_index);

        let new_encoder = match connector_encoders.get(connector_index) {
            Some(encoder) => {
                let crtc = pending.crtc().clone();

                // TODO:
                atomic_state
                    .encoder_states
                    .insert(encoder.index(), DrmEncoderState::new(crtc));

                Arc::downgrade(encoder)
            }
            None => Weak::new(),
        };

        let crtc_changed = !current.crtc().ptr_eq(pending.crtc());
        let encoder_changed = !current.encoder().ptr_eq(&new_encoder);

        atomic_state.needs_modeset |= crtc_changed || encoder_changed;

        pending.set_encoder(new_encoder);
    }

    // TODO: Validate encoder clone compatibility when supporting multiple
    // encoders on the same CRTC.
    Ok(())
}

// TODO: for static display_mode size
pub fn atomic_helper_check_crtc_mode_size(
    modeset_state: &KmsModesetState,
    object_store: &KmsObjectStore,
    atomic_state: &DrmAtomicState,
    fixed_size: DrmSize,
) -> Result<()> {
    for crtc in object_store.static_objects().crtcs() {
        let display_mode = match atomic_state.crtc_states.get(&crtc.index()) {
            Some(pending) => pending.display_mode(),
            None => modeset_state.crtc_state(crtc.index()).display_mode(),
        };

        let Some(display_mode) = display_mode else {
            // Disabled CRTCs have no mode to validate.
            continue;
        };

        if display_mode.hdisplay() != fixed_size.width()
            || display_mode.vdisplay() != fixed_size.height()
        {
            return Err(errno!(
                EINVAL,
                "the display mode size does not match the fixed output size"
            ));
        }
    }

    Ok(())
}

/// Driver limits for validating plane updates during an atomic commit.
pub struct DrmPlaneCheckLimits {
    /// Inclusive minimum source-to-destination size ratio per axis, in unsigned 16.16.
    min_scale: u32,
    /// Inclusive maximum source-to-destination size ratio per axis, in unsigned 16.16.
    max_scale: u32,
    /// Allows a visible plane to cover only part of the CRTC display area.
    can_position: bool,
    /// Allows a plane with a framebuffer to target a disabled CRTC.
    can_update_disabled_crtc: bool,
}

impl DrmPlaneCheckLimits {
    pub fn new(
        min_scale: u32,
        max_scale: u32,
        can_position: bool,
        can_update_disabled_crtc: bool,
    ) -> Self {
        Self {
            min_scale,
            max_scale,
            can_position,
            can_update_disabled_crtc,
        }
    }
}

pub fn atomic_helper_check_plane(
    modeset_state: &KmsModesetState,
    object_store: &KmsObjectStore,
    atomic_state: &mut DrmAtomicState,
    limits: DrmPlaneCheckLimits,
) -> Result<()> {
    for plane in object_store.static_objects().planes() {
        let plane_state = match atomic_state.plane_states.get(&plane.index()) {
            Some(pending) => pending,
            None => modeset_state.plane_state(plane.index()),
        };

        if plane_state.framebuffer().is_none() {
            continue;
        }

        // A plane with a framebuffer always has a live CRTC.
        let crtc = plane_state.crtc().upgrade().unwrap();

        let display_mode = match atomic_state.crtc_states.get(&crtc.index()) {
            Some(pending) => pending.display_mode(),
            None => modeset_state.crtc_state(crtc.index()).display_mode(),
        };

        if display_mode.is_none() && !limits.can_update_disabled_crtc {
            return Err(errno!(
                EINVAL,
                "the plane cannot be updated on a disabled CRTC",
            ));
        }

        let source_rect = plane_state.source_rect();
        let crtc_rect = plane_state.crtc_rect();

        // On each axis, multiply the 16.16 scale limits by the destination extent.
        // Compare against the 16.16 source extent directly to avoid division rounding.
        for (src, dst) in [
            (source_rect.width(), crtc_rect.width()),
            (source_rect.height(), crtc_rect.height()),
        ] {
            let src = u64::from(src.raw());
            let dst = u64::from(dst);
            let min = u64::from(limits.min_scale) * dst;
            let max = u64::from(limits.max_scale) * dst;

            if !(min..=max).contains(&src) {
                return Err(errno!(ERANGE, "the plane scaling ratio is unsupported"));
            }
        }

        // If positioning is unsupported, a visible plane must cover the entire display area.
        if !limits.can_position
            && let Some(mode) = display_mode
        {
            let display_area = DrmCrtcRect::new(0, 0, mode.hdisplay(), mode.vdisplay());
            let left = i64::from(crtc_rect.x());
            let top = i64::from(crtc_rect.y());
            let right = left + i64::from(crtc_rect.width());
            let bottom = top + i64::from(crtc_rect.height());
            let width = i64::from(display_area.width());
            let height = i64::from(display_area.height());

            // Intersect the plane with the display area. With x increasing rightward
            // and y increasing downward, the intersection needs left < right and top < bottom.
            let visible = left.max(0) < right.min(width) && top.max(0) < bottom.min(height);
            if visible && !crtc_rect.contains_rect(&display_area) {
                return Err(errno!(
                    EINVAL,
                    "the plane must cover the entire CRTC display area"
                ));
            }
        }
    }

    Ok(())
}

/// Checks that every enabled CRTC has a primary plane attached.
///
/// Drivers whose hardware requires a primary plane for an enabled CRTC
/// should call this helper. This is not a requirement for all DRM devices.
pub fn atomic_helper_check_crtc_primary_plane(
    modeset_state: &KmsModesetState,
    object_store: &KmsObjectStore,
    atomic_state: &mut DrmAtomicState,
) -> Result<()> {
    let mut crtcs_with_primary_planes = 0u32;

    for plane in object_store.static_objects().planes() {
        if plane.type_() != DrmPlaneType::Primary {
            continue;
        }

        let plane_state = match atomic_state.plane_states.get(&plane.index()) {
            Some(pending) => pending,
            None => modeset_state.plane_state(plane.index()),
        };

        if let Some(crtc) = plane_state.crtc().upgrade() {
            crtcs_with_primary_planes |= 1u32 << crtc.index().get();
        }
    }

    for crtc in object_store.static_objects().crtcs() {
        let is_enabled = match atomic_state.crtc_states.get(&crtc.index()) {
            Some(pending) => pending.is_enabled(),
            None => modeset_state.crtc_state(crtc.index()).is_enabled(),
        };

        if !is_enabled {
            continue;
        }

        let has_primary_plane = crtcs_with_primary_planes & (1u32 << crtc.index().get()) != 0;

        if !has_primary_plane {
            return Err(errno!(EINVAL, "an enabled CRTC must have a primary plane"));
        }
    }

    Ok(())
}

pub fn atomic_helper_commit(
    modeset_state: &mut KmsModesetState,
    object_store: &KmsObjectStore,
    atomic_state: &DrmAtomicState,
) -> Result<()> {
    for (crtc_index, crtc_state) in atomic_state.crtc_states.iter() {
        modeset_state.set_crtc_state(*crtc_index, crtc_state.clone());
    }

    for (plane_index, plane_state) in atomic_state.plane_states.iter() {
        modeset_state.set_plane_state(*plane_index, plane_state.clone());
    }

    for (encoder_index, encoder_state) in atomic_state.encoder_states.iter() {
        modeset_state.set_encoder_state(*encoder_index, encoder_state.clone());
    }

    for (connector_index, connector_state) in atomic_state.connector_states.iter() {
        modeset_state.set_connector_state(*connector_index, connector_state.clone());
    }

    Ok(())
}

pub(crate) fn build_atomic_state(
    modeset_state: &KmsModesetState,
    object_store: &KmsObjectStore,
    atomic_requests: &[DrmAtomicRequest],
) -> Result<DrmAtomicState> {
    let objects = object_store.static_objects();

    let mut atomic_state = DrmAtomicState::default();
    let mut pending_crtc_states: BTreeMap<KmsObjectIndex, PendingCrtcState> = BTreeMap::new();
    let mut pending_connector_states: BTreeMap<KmsObjectIndex, PendingConnectorState> =
        BTreeMap::new();

    for request in atomic_requests {
        if let Some(crtc) = objects.lookup_crtc_by_id(request.object_id) {
            let index = crtc.index();
            let crtc_state = pending_crtc_states.entry(crtc.index()).or_insert_with(|| {
                let crtc_state = modeset_state.crtc_state(index);
                PendingCrtcState {
                    mode_blob: crtc_state.mode_blob().cloned(),
                    is_active: crtc_state.is_active(),
                }
            });

            let crtc_props = crtc.properties();
            // Apply request.object_props to crtc_state.
            for (prop_id, prop_value) in &request.object_props {
                let attachment = crtc_props
                    .attachments()
                    .iter()
                    .find(|attachment| attachment.property().id() == *prop_id)
                    .ok_or_else(|| errno!(ENOENT, "the property is not attached to the CRTC"))?;
                let property = attachment.property();

                if property.is_immutable() {
                    return Err(errno!(EINVAL, "TODO: the property is immutable"));
                }

                let standard_property = objects
                    .standard_property_from_id(property.id())
                    .ok_or(Errno::ENOENT)?;

                match standard_property {
                    DrmStandardProperty::Active => {
                        crtc_state.is_active = match *prop_value {
                            0 => false,
                            1 => true,
                            _ => {
                                return Err(errno!(EINVAL, "the CRTC active value must be 0 or 1"));
                            }
                        };
                    }
                    DrmStandardProperty::ModeId => {
                        let blob_id = u32::try_from(*prop_value).map_err(|_| Errno::EINVAL)?;
                        if blob_id == 0 {
                            crtc_state.mode_blob = None;
                        } else {
                            let blob = object_store
                                .lookup_property_blob(blob_id)
                                .ok_or(Errno::ENOENT)?;
                            DrmDisplayMode::try_from(blob.as_ref())?;
                            crtc_state.mode_blob = Some(blob);
                        }
                    }
                    _ => {
                        return Err(errno!(EINVAL, "TODO"));
                    }
                }
            }

            continue;
        }

        if let Some(plane) = objects.lookup_plane_by_id(request.object_id) {
            let plane_state = atomic_state
                .plane_states
                .entry(plane.index())
                .or_insert_with(|| modeset_state.plane_state(plane.index()).clone());

            let mut source_rect = plane_state.source_rect();
            let mut crtc_rect = plane_state.crtc_rect();
            let mut framebuffer = plane_state.framebuffer().cloned();
            let mut plane_crtc = plane_state.crtc().clone();

            let plane_props = plane.properties();
            for (prop_id, prop_value) in &request.object_props {
                let attachment = plane_props
                    .attachments()
                    .iter()
                    .find(|attachment| attachment.property().id() == *prop_id)
                    .ok_or_else(|| errno!(ENOENT, "the property is not attached to the plane"))?;
                let property = attachment.property();

                if property.is_immutable() {
                    return Err(errno!(EINVAL, "the property is immutable"));
                }

                let standard_property = objects
                    .standard_property_from_id(property.id())
                    .ok_or(Errno::ENOENT)?;

                match standard_property {
                    DrmStandardProperty::SrcX => {
                        let value = u32::try_from(*prop_value).map_err(|_| Errno::EINVAL)?;
                        source_rect.set_x(DrmU16F16::from_raw(value));
                    }
                    DrmStandardProperty::SrcY => {
                        let value = u32::try_from(*prop_value).map_err(|_| Errno::EINVAL)?;
                        source_rect.set_y(DrmU16F16::from_raw(value));
                    }
                    DrmStandardProperty::SrcW => {
                        let value = u32::try_from(*prop_value).map_err(|_| Errno::EINVAL)?;
                        source_rect.set_width(DrmU16F16::from_raw(value));
                    }
                    DrmStandardProperty::SrcH => {
                        let value = u32::try_from(*prop_value).map_err(|_| Errno::EINVAL)?;
                        source_rect.set_height(DrmU16F16::from_raw(value));
                    }
                    DrmStandardProperty::CrtcX => {
                        let value = i32::try_from(*prop_value as i64).map_err(|_| Errno::EINVAL)?;
                        crtc_rect.set_x(value);
                    }
                    DrmStandardProperty::CrtcY => {
                        let value = i32::try_from(*prop_value as i64).map_err(|_| Errno::EINVAL)?;
                        crtc_rect.set_y(value);
                    }
                    DrmStandardProperty::CrtcW => {
                        let value = u32::try_from(*prop_value).map_err(|_| Errno::EINVAL)?;
                        crtc_rect.set_width(value);
                    }
                    DrmStandardProperty::CrtcH => {
                        let value = u32::try_from(*prop_value).map_err(|_| Errno::EINVAL)?;
                        crtc_rect.set_height(value);
                    }
                    DrmStandardProperty::FbId => {
                        let framebuffer_id =
                            u32::try_from(*prop_value).map_err(|_| Errno::EINVAL)?;
                        framebuffer = if framebuffer_id == 0 {
                            None
                        } else {
                            Some(
                                object_store
                                    .dynamic_objects()
                                    .lock()
                                    .lookup_framebuffer(framebuffer_id)
                                    .ok_or(Errno::ENOENT)?,
                            )
                        };
                    }
                    DrmStandardProperty::CrtcId => {
                        let crtc_id = u32::try_from(*prop_value).map_err(|_| Errno::EINVAL)?;
                        plane_crtc = if crtc_id == 0 {
                            Weak::new()
                        } else {
                            let crtc = objects.lookup_crtc_by_id(crtc_id).ok_or(Errno::ENOENT)?;
                            Arc::downgrade(crtc)
                        };
                    }
                    _ => {
                        return Err(errno!(
                            EINVAL,
                            "the property is not supported for atomic plane updates",
                        ));
                    }
                }
            }

            *plane_state = DrmPlaneState::new(source_rect, crtc_rect, framebuffer, plane_crtc);
            continue;
        }

        if let Some(connector) = objects.lookup_connector_by_id(request.object_id) {
            let connector_state = pending_connector_states
                .entry(connector.index())
                .or_insert_with(|| {
                    let connector_state = modeset_state.connector_state(connector.index());
                    let crtc =
                        connector_state
                            .encoder()
                            .upgrade()
                            .map_or_else(Weak::new, |encoder| {
                                modeset_state
                                    .encoder_state(encoder.index())
                                    .current_crtc()
                                    .clone()
                            });

                    PendingConnectorState { crtc }
                });

            let connector_props = connector.properties();
            for (prop_id, prop_value) in &request.object_props {
                let attachment = connector_props
                    .attachments()
                    .iter()
                    .find(|attachment| attachment.property().id() == *prop_id)
                    .ok_or_else(|| {
                        errno!(ENOENT, "the property is not attached to the connector")
                    })?;
                let property = attachment.property();

                if property.is_immutable() {
                    return Err(errno!(EINVAL, "the property is immutable"));
                }

                let standard_property = objects
                    .standard_property_from_id(property.id())
                    .ok_or(Errno::ENOENT)?;

                match standard_property {
                    DrmStandardProperty::CrtcId => {
                        let crtc_id = u32::try_from(*prop_value).map_err(|_| Errno::EINVAL)?;
                        connector_state.crtc = if crtc_id == 0 {
                            Weak::new()
                        } else {
                            let crtc = objects.lookup_crtc_by_id(crtc_id).ok_or(Errno::ENOENT)?;
                            Arc::downgrade(crtc)
                        };
                    }
                    _ => {
                        return Err(errno!(
                            EINVAL,
                            "the property is not supported for atomic connector updates",
                        ));
                    }
                }
            }

            continue;
        }

        return Err(errno!(ENOENT, "the atomic KMS object does not exist"));
    }

    // TODO: Basic check
    for (crtc_index, pending_crtc_state) in pending_crtc_states.iter() {
        if pending_crtc_state.is_active && pending_crtc_state.mode_blob.is_none() {
            return Err(errno!(EINVAL, "an active CRTC must have a display mode"));
        }

        match &pending_crtc_state.mode_blob {
            Some(mode_blob) => {
                let new_crtc_state = DrmCrtcState::new(
                    mode_blob.clone(),
                    DrmDisplayMode::try_from(mode_blob.as_ref()).unwrap(),
                );
                atomic_state.crtc_states.insert(*crtc_index, new_crtc_state);
            }
            None => {
                atomic_state
                    .crtc_states
                    .insert(*crtc_index, DrmCrtcState::default());
            }
        }
    }

    for (plane_index, new_plane_state) in atomic_state.plane_states.iter() {
        let crtc = new_plane_state.crtc().upgrade();
        match (new_plane_state.framebuffer(), crtc.as_ref()) {
            (None, None) => continue,
            (Some(framebuffer), Some(crtc)) => {
                let plane = objects.lookup_plane_by_index(*plane_index).unwrap();

                if !plane.possible_crtcs()[crtc.index().get()] {
                    return Err(errno!(EINVAL, "the plane cannot be attached to the CRTC"));
                }

                if !plane.pixel_formats().contains(&framebuffer.pixel_format()) {
                    return Err(errno!(
                        EINVAL,
                        "the framebuffer format is not supported by the plane",
                    ));
                }

                if !new_plane_state
                    .source_rect()
                    .is_within_pixel_rect(DrmRect::new(
                        0,
                        0,
                        framebuffer.size().width(),
                        framebuffer.size().height(),
                    ))
                {
                    return Err(errno!(
                        EINVAL,
                        "the plane source rectangle exceeds the framebuffer",
                    ));
                }

                // TODO:
                let crtc_rect = new_plane_state.crtc_rect();
                let right = i64::from(crtc_rect.x()) + i64::from(crtc_rect.width());
                let bottom = i64::from(crtc_rect.y()) + i64::from(crtc_rect.height());

                if crtc_rect.width() > i32::MAX as u32
                    || crtc_rect.height() > i32::MAX as u32
                    || right > i64::from(i32::MAX)
                    || bottom > i64::from(i32::MAX)
                {
                    return Err(errno!(
                        ERANGE,
                        "the plane destination rectangle exceeds the signed coordinate range"
                    ));
                }

                // TODO:
                let old_plane_state = modeset_state.plane_state(plane.index());
                let old_crtc = old_plane_state.crtc().upgrade();
                let new_crtc = new_plane_state.crtc().upgrade();

                if let (Some(old_crtc), Some(new_crtc)) = (&old_crtc, &new_crtc)
                    && !Arc::ptr_eq(old_crtc, new_crtc)
                {
                    return Err(errno!(EINVAL, "a plane cannot switch CRTCs directly"));
                }
            }
            _ => {
                return Err(errno!(
                    EINVAL,
                    "the plane must have both a framebuffer and a CRTC",
                ));
            }
        }
    }

    for (connector_index, connector_state) in pending_connector_states.iter() {
        let Some(crtc) = connector_state.crtc.upgrade() else {
            // The connector is detached.
            continue;
        };

        let has_mode = match pending_crtc_states.get(&crtc.index()) {
            Some(crtc_state) => crtc_state.mode_blob.is_some(),
            None => modeset_state
                .crtc_state(crtc.index())
                .display_mode()
                .is_some(),
        };

        if !has_mode {
            return Err(errno!(
                EINVAL,
                "the connector is attached to a CRTC without a display mode",
            ));
        }

        // TODO: update encoder in atomic_check
        if let Some(encoder) = modeset_state
            .connector_state(*connector_index)
            .encoder()
            .upgrade()
        {
            atomic_state
                .encoder_states
                .insert(encoder.index(), DrmEncoderState::default());
        }
        atomic_state.connector_states.insert(
            *connector_index,
            DrmConnectorState::new(Arc::downgrade(&crtc), Weak::new()),
        );
    }

    Ok(atomic_state)
}
