// SPDX-License-Identifier: MPL-2.0

use crate::kms::display_mode::DrmDisplayMode;

#[derive(Clone, Debug, Default)]
pub(crate) struct DrmCrtcState {
    display_mode: Option<DrmDisplayMode>,
    is_enabled: bool,
    #[expect(dead_code)]
    is_active: bool,
}

impl DrmCrtcState {
    pub(crate) fn new(display_mode: DrmDisplayMode) -> Self {
        Self {
            display_mode: Some(display_mode),
            is_enabled: true,
            is_active: true,
        }
    }

    pub(crate) fn display_mode(&self) -> Option<DrmDisplayMode> {
        self.display_mode
    }

    pub(crate) fn is_enabled(&self) -> bool {
        self.is_enabled
    }
}
