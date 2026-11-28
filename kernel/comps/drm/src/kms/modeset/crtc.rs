// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use crate::kms::{display_mode::DrmDisplayMode, objects::property::blob::DrmPropertyBlob};

#[derive(Clone, Debug, Default)]
pub struct DrmCrtcState {
    enable_state: CrtcEnableState,
    // TODO: Add CRTC properties, such as color-management state.
}

impl DrmCrtcState {
    pub(crate) fn new(mode_blob: Arc<DrmPropertyBlob>, display_mode: DrmDisplayMode) -> Self {
        Self {
            enable_state: CrtcEnableState::Enabled {
                mode_blob,
                display_mode,
                is_active: true,
            },
        }
    }

    pub fn display_mode(&self) -> Option<DrmDisplayMode> {
        match self.enable_state {
            CrtcEnableState::Disabled => None,
            CrtcEnableState::Enabled { display_mode, .. } => Some(display_mode),
        }
    }

    pub(crate) fn is_enabled(&self) -> bool {
        match self.enable_state {
            CrtcEnableState::Disabled => false,
            CrtcEnableState::Enabled { .. } => true,
        }
    }

    pub fn is_active(&self) -> bool {
        match self.enable_state {
            CrtcEnableState::Disabled => false,
            CrtcEnableState::Enabled { is_active, .. } => is_active,
        }
    }

    pub(crate) fn mode_blob(&self) -> Option<&Arc<DrmPropertyBlob>> {
        match &self.enable_state {
            CrtcEnableState::Disabled => None,
            CrtcEnableState::Enabled { mode_blob, .. } => Some(mode_blob),
        }
    }
}

/// The CRTC's enable state, requiring a mode when enabled and preventing activity when disabled.
#[derive(Clone, Debug, Default)]
enum CrtcEnableState {
    #[default]
    Disabled,
    Enabled {
        mode_blob: Arc<DrmPropertyBlob>,
        display_mode: DrmDisplayMode,
        is_active: bool,
    },
}
