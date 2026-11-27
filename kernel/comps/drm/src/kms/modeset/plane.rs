// SPDX-License-Identifier: MPL-2.0

use alloc::sync::{Arc, Weak};

use crate::{
    kms::objects::{crtc::DrmCrtc, framebuffer::DrmFramebuffer},
    utils::{DrmCrtcRect, DrmSourceRect},
};

#[derive(Clone, Debug, Default)]
pub(crate) struct DrmPlaneState {
    source_rect: DrmSourceRect,
    #[expect(dead_code)]
    crtc_rect: DrmCrtcRect,

    /// Keeps the bound framebuffer alive independently of its creating file.
    framebuffer: Option<Arc<DrmFramebuffer>>,
    crtc: Weak<DrmCrtc>,
}

impl DrmPlaneState {
    pub(crate) fn new(
        source_rect: DrmSourceRect,
        crtc_rect: DrmCrtcRect,
        framebuffer: Option<Arc<DrmFramebuffer>>,
        crtc: Weak<DrmCrtc>,
    ) -> Self {
        Self {
            source_rect,
            crtc_rect,
            framebuffer,
            crtc,
        }
    }

    pub(crate) fn source_rect(&self) -> DrmSourceRect {
        self.source_rect
    }

    pub(crate) fn framebuffer(&self) -> Option<&Arc<DrmFramebuffer>> {
        self.framebuffer.as_ref()
    }

    pub(crate) fn crtc(&self) -> &Weak<DrmCrtc> {
        &self.crtc
    }
}
