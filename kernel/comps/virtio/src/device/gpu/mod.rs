// SPDX-License-Identifier: MPL-2.0

use aster_drm::DrmError;

use crate::device::{VirtioDeviceError, gpu::header::VirtioGpuCtrlType};

mod config;
pub mod device;
mod gem;
mod header;
mod ioctl;

#[derive(Debug)]
pub(super) enum VirtioGpuCommandError {
    ResourceAlloc(ostd::Error),
    QueueUnavailable,
    Timeout,
    InvalidResponseType {
        expected: VirtioGpuCtrlType,
        actual: VirtioGpuCtrlType,
    },
    InvalidValue,
    Device(VirtioGpuDeviceError),
}

#[derive(Debug)]
pub(super) enum VirtioGpuDeviceError {
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

impl From<VirtioGpuCommandError> for DrmError {
    fn from(error: VirtioGpuCommandError) -> Self {
        match error {
            VirtioGpuCommandError::ResourceAlloc(_) => Self::NoMemory,
            VirtioGpuCommandError::QueueUnavailable
            | VirtioGpuCommandError::InvalidResponseType { .. }
            | VirtioGpuCommandError::InvalidValue => Self::Invalid,
            VirtioGpuCommandError::Timeout => Self::Busy,
            VirtioGpuCommandError::Device(error) => error.into(),
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

impl From<VirtioGpuCommandError> for VirtioDeviceError {
    fn from(error: VirtioGpuCommandError) -> VirtioDeviceError {
        match error {
            VirtioGpuCommandError::ResourceAlloc(error) => VirtioDeviceError::ResourceAlloc(error),
            VirtioGpuCommandError::QueueUnavailable => VirtioDeviceError::InvalidQueueArgs,
            VirtioGpuCommandError::Timeout
            | VirtioGpuCommandError::InvalidResponseType { .. }
            | VirtioGpuCommandError::InvalidValue
            | VirtioGpuCommandError::Device(_) => VirtioDeviceError::UnsupportedConfig,
        }
    }
}
