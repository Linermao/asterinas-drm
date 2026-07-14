// SPDX-License-Identifier: MPL-2.0

use aster_drm::DrmError;

use crate::device::gpu::queue::header::VirtioGpuCtrlType;

mod config;
pub mod device;
mod gem;
mod ioctl;
mod queue;

#[derive(Debug)]
pub enum VirtioGpuDeviceError {
    Unspec,
    OutOfMemory,
    InvalidScanoutId,
    InvalidResourceId,
    InvalidContextId,
    InvalidParameter,
}

impl VirtioGpuDeviceError {
    pub(super) fn from_ctrl_type(type_: VirtioGpuCtrlType) -> Option<Self> {
        match type_ {
            VirtioGpuCtrlType::RespErrUnspec => Some(Self::Unspec),
            VirtioGpuCtrlType::RespErrOutOfMemory => Some(Self::OutOfMemory),
            VirtioGpuCtrlType::RespErrInvalidScanoutId => Some(Self::InvalidScanoutId),
            VirtioGpuCtrlType::RespErrInvalidResourceId => Some(Self::InvalidResourceId),
            VirtioGpuCtrlType::RespErrInvalidContextId => Some(Self::InvalidContextId),
            VirtioGpuCtrlType::RespErrInvalidParameter => Some(Self::InvalidParameter),
            _ => None,
        }
    }
}

impl From<VirtioGpuDeviceError> for DrmError {
    fn from(error: VirtioGpuDeviceError) -> Self {
        match error {
            VirtioGpuDeviceError::OutOfMemory => Self::NoMemory,
            VirtioGpuDeviceError::InvalidScanoutId
            | VirtioGpuDeviceError::InvalidResourceId
            | VirtioGpuDeviceError::InvalidContextId => Self::NotFound,
            VirtioGpuDeviceError::InvalidParameter | VirtioGpuDeviceError::Unspec => Self::Invalid,
        }
    }
}
