use alloc::{sync::Arc, vec::Vec};
use core::fmt::Debug;

use hashbrown::HashMap;

use crate::drm::{
    DrmDevice, DrmError,
    atomic::vblank::{DrmPendingVblankEvent, PageFlipEvent, VblankCallback},
    drm_modes::DrmDisplayMode,
    ioctl::{DrmModeCrtc, DrmModeCrtcPageFlip},
    mode_config::ObjectId,
    mode_object::{
        DrmObject, DrmObjectType,
        connector::DrmConnector,
        crtc::DrmCrtc,
        encoder::DrmEncoder,
        framebuffer::DrmFramebuffer,
        plane::{DrmPlane, PlaneState},
        property::{DrmModeBlob, DrmProperty, DrmPropertyType},
    },
};

pub mod vblank;

bitflags::bitflags! {
    pub struct DrmAtomicFlags: u32 {
        const PAGE_FLIP_EVENT    = 0x0001;
        const PAGE_FLIP_ASYNC    = 0x0002;

        const TEST_ONLY          = 0x0100;
        const NONBLOCK           = 0x0200;
        const ALLOW_MODESET      = 0x0400;
    }
}

#[derive(Debug, Default)]
struct PendingPlaneState {
    crtc_id: Option<ObjectId>,
    fb_id: Option<ObjectId>,
    src_x: Option<u32>,
    src_y: Option<u32>,
    src_w: Option<u32>,
    src_h: Option<u32>,

    crtc_x: Option<u32>,
    crtc_y: Option<u32>,
    crtc_w: Option<u32>,
    crtc_h: Option<u32>,
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

        let config = dev.mode_config().lock();
        for (object_id, prop_count) in object_ids.iter().zip(prop_counts.iter()) {
            let object = config
                .get_object(*object_id, DrmObjectType::Any)
                .ok_or(DrmError::NotFound)?;

            for _ in 0..*prop_count {
                let prop_id = prop_ids_iter.next().ok_or(DrmError::Invalid)?;
                let prop_value = prop_values_iter.next().ok_or(DrmError::Invalid)?;

                if !object.get_properties().contains_key(&prop_id) {
                    return Err(DrmError::Invalid);
                }
                let property = config
                    .get_object_with::<DrmProperty>(prop_id)
                    .ok_or(DrmError::NotFound)?;

                match object {
                    DrmObject::Plane(plane) => match property.type_() {
                        DrmPropertyType::CrtcId => {
                            let new_crtc_id = prop_value as u32;
                            let old_crtc_id = plane.crtc_id();

                            if old_crtc_id != Some(new_crtc_id)
                                && config.get_object_with::<dyn DrmCrtc>(new_crtc_id).is_some()
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
                            let _blob = config
                                .get_object_with::<DrmModeBlob>(blob_id)
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
                                    config
                                        .collect_object_ids(
                                            DrmObjectType::Encoder,
                                            Some(connector.possible_encoders()),
                                        )
                                        .first()
                                        .copied()
                                })
                                .ok_or(DrmError::NotFound)?;

                            let old_crtc_id = config
                                .get_object_with::<dyn DrmEncoder>(encoder_id)
                                .map(|enc| enc.crtc_id())
                                .unwrap_or(None);

                            if old_crtc_id != Some(new_crtc_id)
                                && config.get_object_with::<dyn DrmCrtc>(new_crtc_id).is_some()
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

pub trait DrmAtomicHelper: DrmDevice + Debug + Sync + Send {
    fn atomic_helper_set_config(
        &self,
        crtc_resp: &DrmModeCrtc,
        connector_ids: Vec<ObjectId>,
    ) -> Result<(), DrmError> {
        let mut pending_state = DrmAtomicPendingState::new();

        {
            let config = self.mode_config().lock();
            let crtc_state = PendingCrtcState {
                active: Some(true),
                display_mode: None,
            };
            let plane_state = PendingPlaneState {
                crtc_id: Some(crtc_resp.crtc_id),
                fb_id: Some(crtc_resp.fb_id),
                src_x: None,
                src_y: None,
                src_w: None,
                src_h: None,
                crtc_x: Some(crtc_resp.x),
                crtc_y: Some(crtc_resp.y),
                crtc_w: None,
                crtc_h: None,
            };

            let crtc = config
                .get_object_with::<dyn DrmCrtc>(crtc_resp.crtc_id)
                .ok_or(DrmError::NotFound)?;

            let plane_id = crtc.primary_plane_id();

            pending_state
                .crtc_states
                .insert(crtc_resp.crtc_id, crtc_state);
            pending_state.plane_states.insert(plane_id, plane_state);

            for id in connector_ids {
                let connector = config
                    .get_object_with::<dyn DrmConnector>(id)
                    .ok_or(DrmError::NotFound)?;

                // Get first possible encoder
                // TODO:
                let encoder_id = config
                    .collect_object_ids(DrmObjectType::Encoder, Some(connector.possible_encoders()))
                    .first()
                    .copied()
                    .ok_or(DrmError::NotFound)?;

                let connector_state = PendingConnectorState {
                    crtc_id: Some(crtc_resp.crtc_id),
                    encoder_id: Some(encoder_id),
                };

                pending_state.connector_states.insert(id, connector_state);
            }
        }

        self.atomic_helper_commit(false, &mut pending_state, None)
    }

    fn atomic_helper_pageflip(
        &self,
        page_flip: &DrmModeCrtcPageFlip,
        vblank_callback: Arc<dyn VblankCallback>,
        _target: Option<u32>,
    ) -> Result<(), DrmError> {
        let mut pending_state = DrmAtomicPendingState::new();

        {
            let config = self.mode_config().lock();
            let crtc_state = PendingCrtcState {
                active: Some(true),
                display_mode: None,
            };
            let plane_state = PendingPlaneState {
                crtc_id: Some(page_flip.crtc_id),
                fb_id: Some(page_flip.fb_id),
                src_x: None,
                src_y: None,
                src_w: None,
                src_h: None,
                crtc_x: None,
                crtc_y: None,
                crtc_w: None,
                crtc_h: None,
            };

            let crtc = config
                .get_object_with::<dyn DrmCrtc>(page_flip.crtc_id)
                .ok_or(DrmError::NotFound)?;

            let plane_id = crtc.primary_plane_id();

            pending_state
                .crtc_states
                .insert(page_flip.crtc_id, crtc_state);
            pending_state.plane_states.insert(plane_id, plane_state);
        }

        let page_flip_event = PageFlipEvent::new(page_flip.user_data, vblank_callback);
        self.atomic_helper_commit(true, &mut pending_state, Some(page_flip_event))?;

        Ok(())
    }

    fn atomic_helper_dirty_framebuffer(&self, fb_id: ObjectId) -> Result<(), DrmError> {
        let mut affected_crtcs = Vec::new();
        {
            let config = self.mode_config().lock();
            for plane_id in config.collect_object_ids(DrmObjectType::Plane, None) {
                let plane = config
                    .get_object_with::<dyn DrmPlane>(plane_id)
                    .ok_or(DrmError::NotFound)?;

                if plane.fb_id() == Some(fb_id) {
                    if let Some(crtc_id) = plane.crtc_id() {
                        affected_crtcs.push(crtc_id);
                    }
                }
            }
        };

        if affected_crtcs.is_empty() {
            return Err(DrmError::NotFound);
        }

        for crtc_id in affected_crtcs {
            self.atomic_flush(crtc_id)?;
        }

        Ok(())
    }

    fn atomic_helper_commit(
        &self,
        nonblock: bool,
        pending_state: &mut DrmAtomicPendingState,
        page_flip_event: Option<PageFlipEvent>,
    ) -> Result<(), DrmError> {
        if nonblock {}

        {
            let config = self.mode_config().lock();
            for (id, state) in pending_state.plane_states.iter() {
                let plane = config
                    .get_object_with::<dyn DrmPlane>(*id)
                    .ok_or(DrmError::NotFound)?;

                let old_crtc = plane.crtc_id();

                if let Some(new_crtc) = state.crtc_id {
                    plane.set_crtc_id(Some(new_crtc));
                }
                if let Some(new_fb) = state.fb_id {
                    plane.set_fb_id(Some(new_fb));
                }

                let new_crtc = plane.crtc_id();

                if state.crtc_id.is_some() || state.fb_id.is_some() {
                    if let Some(id) = old_crtc {
                        pending_state.affected_crtcs.push(id);
                    }
                    if let Some(id) = new_crtc
                        && new_crtc != old_crtc
                    {
                        pending_state.affected_crtcs.push(id);
                    }
                }
            }

            for (id, state) in pending_state.crtc_states.iter() {
                let crtc = config
                    .get_object_with::<dyn DrmCrtc>(*id)
                    .ok_or(DrmError::NotFound)?;

                if state.active == Some(false) {
                    let primary = crtc.primary_plane();

                    primary.set_crtc_id(None);
                    primary.set_fb_id(None);

                    crtc.set_active(false);
                } else if state.active == Some(true) {
                    crtc.set_active(true);
                }
            }

            for (id, state) in pending_state.connector_states.iter() {
                let connector = config
                    .get_object_with::<dyn DrmConnector>(*id)
                    .ok_or(DrmError::NotFound)?;

                if let Some(encoder_id) = state.encoder_id {
                    connector.set_encoder_id(Some(encoder_id));
                    let encoder = config
                        .get_object_with::<dyn DrmEncoder>(encoder_id)
                        .ok_or(DrmError::NotFound)?;
                    encoder.set_crtc_id(state.crtc_id);
                }
            }
        }

        if let Some(page_flip_event) = page_flip_event {
            for affected_crtc_id in &pending_state.affected_crtcs {
                let config = self.mode_config().lock();
                let crtc = config
                    .get_object_with::<dyn DrmCrtc>(*affected_crtc_id)
                    .ok_or(DrmError::NotFound)?;

                let vblank_state = crtc.vblank_state().lock();
                vblank_state.queue_event(DrmPendingVblankEvent::new(
                    page_flip_event.user_data(),
                    *affected_crtc_id,
                    page_flip_event.vblank_callback(),
                ));
            }
        }

        self.atomic_helper_commit_tail(pending_state)
    }

    fn atomic_helper_commit_tail(
        &self,
        pending_state: &mut DrmAtomicPendingState,
    ) -> Result<(), DrmError> {
        for affected_crtc_id in &pending_state.affected_crtcs {
            self.atomic_flush(*affected_crtc_id)?;
        }

        Ok(())
    }

    fn atomic_flush(&self, crtc_id: ObjectId) -> Result<(), DrmError>;
}
