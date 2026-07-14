// SPDX-License-Identifier: MPL-2.0

use aster_drm::DrmError;

use crate::device::{
    VirtioDeviceError,
    gpu::{VirtioGpuDeviceError, queue::header::VirtioGpuCtrlType},
};

pub(super) mod control;
pub(super) mod header;

#[derive(Debug)]
pub enum VirtioGpuCommandError {
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
