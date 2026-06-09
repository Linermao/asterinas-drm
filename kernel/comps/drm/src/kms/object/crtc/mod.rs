// SPDX-License-Identifier: MPL-2.0

use core::fmt::Debug;

use ostd::sync::Mutex;

use crate::{
    display::DrmDisplayMode,
    kms::object::{DrmKmsObject, DrmKmsObjectCast, KmsObjectId},
};

#[derive(Debug, Default)]
pub struct DrmCrtcState {
    display_mode: Option<DrmDisplayMode>,
    enable: bool,
    active: bool,
}

impl DrmCrtcState {
    pub fn snapshot(&self) -> DrmCrtcSnapshot {
        DrmCrtcSnapshot {
            display_mode: self.display_mode,
            enable: self.enable,
            active: self.active,
        }
    }

    pub fn set_display_mode(&mut self, display_mode: Option<DrmDisplayMode>) {
        self.display_mode = display_mode;
    }

    pub fn set_enable(&mut self, enable: bool) {
        self.enable = enable;
    }

    pub fn set_active(&mut self, active: bool) {
        self.active = active;
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct DrmCrtcSnapshot {
    display_mode: Option<DrmDisplayMode>,
    enable: bool,
    active: bool,
}

impl DrmCrtcSnapshot {
    pub fn display_mode(&self) -> Option<DrmDisplayMode> {
        self.display_mode
    }

    pub fn enable(&self) -> bool {
        self.enable
    }

    pub fn active(&self) -> bool {
        self.active
    }
}

#[derive(Debug)]
pub struct DrmCrtc {
    state: Mutex<DrmCrtcState>,
    gamma_size: u32,
    primary_plane_id: KmsObjectId,
    cursor_plane_id: Option<KmsObjectId>,
}

impl DrmCrtc {
    pub fn new(
        gamma_size: u32,
        primary_plane_id: KmsObjectId,
        cursor_plane_id: Option<KmsObjectId>,
    ) -> Self {
        Self {
            state: Mutex::new(DrmCrtcState::default()),
            gamma_size,
            primary_plane_id,
            cursor_plane_id,
        }
    }

    pub fn state(&self) -> &Mutex<DrmCrtcState> {
        &self.state
    }

    pub fn gamma_size(&self) -> u32 {
        self.gamma_size
    }

    pub fn primary_plane_id(&self) -> KmsObjectId {
        self.primary_plane_id
    }

    pub fn cursor_plane_id(&self) -> Option<KmsObjectId> {
        self.cursor_plane_id
    }

    pub fn state_snapshot(&self) -> DrmCrtcSnapshot {
        self.state.lock().snapshot()
    }
}

impl DrmKmsObjectCast for DrmCrtc {
    fn cast(obj: &DrmKmsObject) -> Option<&Self> {
        if let DrmKmsObject::Crtc(crtc) = obj {
            Some(crtc)
        } else {
            None
        }
    }
}
