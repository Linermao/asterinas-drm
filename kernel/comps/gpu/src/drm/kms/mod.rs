use alloc::{sync::Arc, vec::Vec};
use core::fmt::Debug;

use crate::drm::{
    DrmError,
    ioctl::{DrmModeCrtc, DrmModeCrtcPageFlip},
    kms::vblank::VblankCallback, objects::ObjectId,
};

pub mod vblank;

pub trait DrmKmsOps: Debug + Send + Sync {
    fn set_crtc(
        &self,
        crtc_resp: &DrmModeCrtc,
        connector_ids: Vec<ObjectId>,
    ) -> Result<(), DrmError>;
    fn page_flip(
        &self,
        page_flip: &DrmModeCrtcPageFlip,
        vblank_callback: Arc<dyn VblankCallback>,
        target: Option<u32>,
    ) -> Result<(), DrmError>;
    fn dirty_framebuffer(&self, fb_id: ObjectId) -> Result<(), DrmError>;
    fn update_connector_modes_and_status(&self, connector_id: ObjectId) -> Result<(), DrmError>;
}
