// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Weak;

use int_to_c_enum::TryFromInt;

use crate::kms::objects::encoder::DrmEncoder;

#[derive(Clone, Debug, Default)]
pub(crate) struct DrmConnectorState {
    encoder: Weak<DrmEncoder>,
    dpms: DrmDpms,
}

impl DrmConnectorState {
    pub(crate) fn new(encoder: Weak<DrmEncoder>) -> Self {
        Self {
            encoder,
            dpms: DrmDpms::On,
        }
    }

    pub(crate) fn encoder(&self) -> &Weak<DrmEncoder> {
        &self.encoder
    }

    pub(crate) fn dpms(&self) -> DrmDpms {
        self.dpms
    }

    pub(crate) fn set_dpms(&mut self, dpms: DrmDpms) {
        self.dpms = dpms;
    }
}

/// The legacy connector power states defined by `DRM_MODE_DPMS_*`.
#[repr(u64)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, TryFromInt)]
pub(crate) enum DrmDpms {
    #[default]
    On = 0,
    Standby = 1,
    Suspend = 2,
    Off = 3,
}
