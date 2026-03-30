use alloc::{sync::Arc, vec::Vec};
use core::fmt::Debug;

use hashbrown::HashMap;

use crate::drm::{
    DrmDevice, DrmError,
    drm_modes::DrmDisplayMode,
    kms::vblank::PageFlipEvent,
    objects::{
        DrmObject, DrmObjectType, ObjectId,
        crtc::DrmCrtc,
        encoder::DrmEncoder,
        property::{DrmModeBlob, DrmProperty, DrmPropertyType},
    },
};

pub mod helper;

bitflags::bitflags! {
    pub struct DrmAtomicFlags: u32 {
        const PAGE_FLIP_EVENT    = 0x0001;
        const PAGE_FLIP_ASYNC    = 0x0002;

        const TEST_ONLY          = 0x0100;
        const NONBLOCK           = 0x0200;
        const ALLOW_MODESET      = 0x0400;
    }
}

pub trait DrmAtomicOps: Debug + Send + Sync {
    fn atomic_commit(
        &self,
        nonblock: bool,
        pending_state: &mut DrmAtomicPendingState,
        page_flip_event: Option<PageFlipEvent>,
    ) -> Result<(), DrmError>;
    fn atomic_commit_tail(&self, pending_state: &mut DrmAtomicPendingState)
    -> Result<(), DrmError>;
}

#[derive(Debug, Default)]
struct PendingPlaneState {
    crtc_id: Option<ObjectId>,
    fb_id: Option<ObjectId>,
}

#[derive(Debug, Default)]
struct PendingCrtcState {
    active: Option<bool>,
    display_mode: Option<DrmDisplayMode>,
}

#[derive(Debug, Default)]
struct PendingConnectorState {
    crtc_id: Option<ObjectId>,
    encoder_id: Option<ObjectId>,
}

#[derive(Debug)]
pub struct DrmAtomicPendingState {
    plane_states: HashMap<ObjectId, PendingPlaneState>,
    crtc_states: HashMap<ObjectId, PendingCrtcState>,
    connector_states: HashMap<ObjectId, PendingConnectorState>,
    affected_crtcs: Vec<ObjectId>,
}

impl DrmAtomicPendingState {
    pub fn new() -> Self {
        Self {
            plane_states: HashMap::new(),
            crtc_states: HashMap::new(),
            connector_states: HashMap::new(),
            affected_crtcs: Vec::new(),
        }
    }

    pub fn init(
        &mut self,
        dev: Arc<dyn DrmDevice>,
        object_ids: Vec<ObjectId>,
        prop_counts: Vec<u32>,
        prop_ids: Vec<ObjectId>,
        prop_values: Vec<u64>,
    ) -> Result<bool, DrmError> {
        let mut requires_modeset = false;

        let mut prop_ids_iter = prop_ids.into_iter();
        let mut prop_values_iter = prop_values.into_iter();

        let plane_states = &mut self.plane_states;
        let crtc_states = &mut self.crtc_states;
        let connector_states = &mut self.connector_states;

        let objects = dev.objects().lock();
        for (object_id, prop_count) in object_ids.iter().zip(prop_counts.iter()) {
            let object = objects
                .get_object(*object_id, DrmObjectType::Any)
                .ok_or(DrmError::NotFound)?;

            for _ in 0..*prop_count {
                let prop_id = prop_ids_iter.next().ok_or(DrmError::Invalid)?;
                let prop_value = prop_values_iter.next().ok_or(DrmError::Invalid)?;

                if !object.get_properties().contains_key(&prop_id) {
                    return Err(DrmError::Invalid);
                }
                let property = objects
                    .get_object_by_id::<DrmProperty>(prop_id)
                    .ok_or(DrmError::NotFound)?;

                match object {
                    DrmObject::Plane(plane) => match property.type_() {
                        DrmPropertyType::CrtcId => {
                            let new_crtc_id = prop_value as u32;
                            let old_crtc_id = plane.crtc_id();

                            if old_crtc_id != Some(new_crtc_id)
                                && objects
                                    .get_object_by_id::<dyn DrmCrtc>(new_crtc_id)
                                    .is_some()
                            {
                                plane_states.entry(*object_id).or_default().crtc_id =
                                    Some(new_crtc_id);
                                requires_modeset = true;
                            }
                        }
                        DrmPropertyType::FbId => {
                            let new_fb_id = prop_value as u32;
                            plane_states.entry(*object_id).or_default().fb_id = Some(new_fb_id);
                        }
                        _ => {}
                    },
                    DrmObject::Crtc(crtc) => match property.type_() {
                        DrmPropertyType::Active => {
                            if prop_value > 1 {
                                return Err(DrmError::Invalid);
                            }
                            if crtc.active() != (prop_value != 0) {
                                crtc_states.entry(*object_id).or_default().active =
                                    Some(prop_value != 0);
                                requires_modeset = true;
                            }
                        }
                        DrmPropertyType::ModeId => {
                            let blob_id = prop_value as u32;
                            let _blob = objects
                                .get_object_by_id::<DrmModeBlob>(blob_id)
                                .ok_or(DrmError::NotFound)?;
                            // TODO: convert blob to display info
                            crtc_states.entry(*object_id).or_default().display_mode =
                                Some(DrmDisplayMode::default());
                        }
                        _ => {}
                    },
                    DrmObject::Connector(connector) => match property.type_() {
                        DrmPropertyType::CrtcId => {
                            let new_crtc_id: u32 = prop_value as u32;
                            let encoder_id = connector
                                .encoder_id()
                                .or_else(|| {
                                    objects
                                        .collect_object_ids(
                                            DrmObjectType::Encoder,
                                            Some(connector.possible_encoders()),
                                        )
                                        .first()
                                        .copied()
                                })
                                .ok_or(DrmError::NotFound)?;

                            let old_crtc_id = objects
                                .get_object_by_id::<dyn DrmEncoder>(encoder_id)
                                .map(|enc| enc.crtc_id())
                                .unwrap_or(None);

                            if old_crtc_id != Some(new_crtc_id)
                                && objects
                                    .get_object_by_id::<dyn DrmCrtc>(new_crtc_id)
                                    .is_some()
                            {
                                connector_states.entry(*object_id).or_default().crtc_id =
                                    Some(new_crtc_id);
                                connector_states.entry(*object_id).or_default().encoder_id =
                                    Some(encoder_id);
                                requires_modeset = true;
                            }
                        }
                        _ => {}
                    },
                    _ => {}
                }
            }
        }

        Ok(requires_modeset)
    }
}
