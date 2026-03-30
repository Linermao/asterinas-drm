use alloc::{sync::Arc, vec::Vec};
use core::fmt::Debug;

use crate::drm::{
    DrmDevice, DrmError,
    atomic::{DrmAtomicPendingState, PendingConnectorState, PendingCrtcState, PendingPlaneState},
    ioctl::{DrmModeCrtc, DrmModeCrtcPageFlip},
    kms::vblank::{DrmPendingVblankEvent, PageFlipEvent, VblankCallback},
    objects::{
        DrmObjectType, ObjectId, connector::DrmConnector, crtc::DrmCrtc, encoder::DrmEncoder,
        plane::DrmPlane,
    },
};

pub trait DrmAtomicHelper: DrmDevice + Debug + Sync + Send {
    fn atomic_helper_set_config(
        &self,
        crtc_resp: &DrmModeCrtc,
        connector_ids: Vec<ObjectId>,
    ) -> Result<(), DrmError> {
        let mut pending_state = DrmAtomicPendingState::new();

        {
            let objects = self.objects().lock();
            let crtc_state = PendingCrtcState {
                active: Some(true),
                display_mode: None,
            };
            let plane_state = PendingPlaneState {
                crtc_id: Some(crtc_resp.crtc_id),
                fb_id: Some(crtc_resp.fb_id),
            };

            let crtc = objects
                .get_object_by_id::<dyn DrmCrtc>(crtc_resp.crtc_id)
                .ok_or(DrmError::NotFound)?;

            let plane_id = crtc.primary_plane_id();

            pending_state
                .crtc_states
                .insert(crtc_resp.crtc_id, crtc_state);
            pending_state.plane_states.insert(plane_id, plane_state);

            for id in connector_ids {
                let connector = objects
                    .get_object_by_id::<dyn DrmConnector>(id)
                    .ok_or(DrmError::NotFound)?;

                // Get first possible encoder
                // TODO:
                let encoder_id = objects
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
            let objects = self.objects().lock();
            let crtc_state = PendingCrtcState {
                active: Some(true),
                display_mode: None,
            };
            let plane_state = PendingPlaneState {
                crtc_id: Some(page_flip.crtc_id),
                fb_id: Some(page_flip.fb_id),
            };

            let crtc = objects
                .get_object_by_id::<dyn DrmCrtc>(page_flip.crtc_id)
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
            let objects = self.objects().lock();
            for plane_id in objects.collect_object_ids(DrmObjectType::Plane, None) {
                let plane = objects
                    .get_object_by_id::<dyn DrmPlane>(plane_id)
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
            let objects = self.objects().lock();
            for (id, state) in pending_state.plane_states.iter() {
                let plane = objects
                    .get_object_by_id::<dyn DrmPlane>(*id)
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
                let crtc = objects
                    .get_object_by_id::<dyn DrmCrtc>(*id)
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
                let connector = objects
                    .get_object_by_id::<dyn DrmConnector>(*id)
                    .ok_or(DrmError::NotFound)?;

                if let Some(encoder_id) = state.encoder_id {
                    connector.set_encoder_id(Some(encoder_id));
                    let encoder = objects
                        .get_object_by_id::<dyn DrmEncoder>(encoder_id)
                        .ok_or(DrmError::NotFound)?;
                    encoder.set_crtc_id(state.crtc_id);
                }
            }
        }

        if let Some(page_flip_event) = page_flip_event {
            for affected_crtc_id in &pending_state.affected_crtcs {
                let objects = self.objects().lock();
                let crtc = objects
                    .get_object_by_id::<dyn DrmCrtc>(*affected_crtc_id)
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
