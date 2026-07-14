// SPDX-License-Identifier: MPL-2.0

use int_to_c_enum::TryFromInt;

const DRM_COMMAND_BASE: u8 = 0x40;
const DRM_IOCTL_BASE: u8 = b'd';
const IOC_READ_WRITE: u32 = 3;
pub(super) const VIRTIO_GPU_MAX_CAPSET_ID: u32 = 63;
pub(super) const VIRTIO_GPU_CONTEXT_INIT_CAPSET_ID_MASK: u32 = 0xff;
pub(super) const VIRTGPU_MAX_RINGS: u32 = 64;
pub(super) const VIRTGPU_DEBUG_NAME_MAX_LEN: usize = 65;
pub(super) const VIRTGPU_EXECBUFFER_MAX_COMMAND_SIZE: usize = 256 * 1024;
pub(super) const VIRTGPU_EXECBUFFER_MAX_BO_HANDLES: usize = 4096;
// const VIRTGPU_EXECBUFFER_MAX_SYNCOBJS: usize = 4096;

const fn drm_iowr<T>(nr: u8) -> u32 {
    (IOC_READ_WRITE << 30)
        | ((size_of::<T>() as u32) << 16)
        | ((DRM_IOCTL_BASE as u32) << 8)
        | (nr as u32)
}

// TODO: use ioc! to unity.
pub(super) const DRM_IOCTL_VIRTGPU_MAP: u32 = drm_iowr::<DrmVirtgpuMap>(DRM_COMMAND_BASE + 0x01);
pub(super) const DRM_IOCTL_VIRTGPU_EXECBUFFER: u32 =
    drm_iowr::<DrmVirtgpuExecbuffer>(DRM_COMMAND_BASE + 0x02);
pub(super) const DRM_IOCTL_VIRTGPU_GETPARAM: u32 =
    drm_iowr::<DrmVirtgpuGetparam>(DRM_COMMAND_BASE + 0x03);
pub(super) const DRM_IOCTL_VIRTGPU_RESOURCE_CREATE: u32 =
    drm_iowr::<DrmVirtgpuResourceCreate>(DRM_COMMAND_BASE + 0x04);
pub(super) const DRM_IOCTL_VIRTGPU_RESOURCE_INFO: u32 =
    drm_iowr::<DrmVirtgpuResourceInfo>(DRM_COMMAND_BASE + 0x05);
pub(super) const DRM_IOCTL_VIRTGPU_TRANSFER_FROM_HOST: u32 =
    drm_iowr::<DrmVirtgpu3dTransferFromHost>(DRM_COMMAND_BASE + 0x06);
pub(super) const DRM_IOCTL_VIRTGPU_TRANSFER_TO_HOST: u32 =
    drm_iowr::<DrmVirtgpu3dTransferToHost>(DRM_COMMAND_BASE + 0x07);
pub(super) const DRM_IOCTL_VIRTGPU_WAIT: u32 =
    drm_iowr::<DrmVirtgpu3dWait>(DRM_COMMAND_BASE + 0x08);
pub(super) const DRM_IOCTL_VIRTGPU_GET_CAPS: u32 =
    drm_iowr::<DrmVirtgpuGetCaps>(DRM_COMMAND_BASE + 0x09);
pub(super) const DRM_IOCTL_VIRTGPU_RESOURCE_CREATE_BLOB: u32 =
    drm_iowr::<DrmVirtgpuResourceCreateBlob>(DRM_COMMAND_BASE + 0x0a);
pub(super) const DRM_IOCTL_VIRTGPU_CONTEXT_INIT: u32 =
    drm_iowr::<DrmVirtgpuContextInit>(DRM_COMMAND_BASE + 0x0b);

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, Pod)]
pub(super) struct DrmVirtgpuMap {
    pub offset: u64,
    pub handle: u32,
    pub pad: u32,
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, Pod)]
pub(super) struct DrmVirtgpuExecbuffer {
    pub flags: u32,
    pub size: u32,
    pub command: u64,
    pub bo_handles: u64,
    pub num_bo_handles: u32,
    pub fence_fd: i32,
    pub ring_idx: u32,
    pub syncobj_stride: u32,
    pub num_in_syncobjs: u32,
    pub num_out_syncobjs: u32,
    pub in_syncobjs: u64,
    pub out_syncobjs: u64,
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, Pod)]
pub(super) struct DrmVirtgpuExecbufferSyncobj {
    pub handle: u32,
    pub flags: u32,
    pub point: u64,
}

bitflags::bitflags! {
    pub(super) struct VirtioGpuExecbufferFlags: u32 {
        const FENCE_FD_IN = 1 << 0;
        const FENCE_FD_OUT = 1 << 1;
        const RING_IDX = 1 << 2;
    }
}

bitflags::bitflags! {
    pub(super) struct VirtioGpuExecbufferSyncobjFlags: u32 {
        const RESET = 1 << 0;
    }
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, Pod)]
pub(super) struct DrmVirtgpuGetparam {
    pub param: u64,
    pub value: u64,
}

#[repr(u64)]
#[derive(Debug, PartialEq, Eq, TryFromInt)]
pub(super) enum VirtioGpuGetParam {
    CapsetId = 1,
    NumRings = 2,
    PollRingsMask = 3,
    DebugName = 4,
}

#[repr(u64)]
#[derive(Debug, PartialEq, Eq, TryFromInt)]
pub(super) enum VirtioGpuParam {
    Features3D = 1,
    CapsetQueryFix = 2,
    ResourceBlob = 3,
    HostVisible = 4,
    CrossDevice = 5,
    ContextInit = 6,
    SupportedCapsetIds = 7,
    ExplicitDebugName = 8,
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, Pod)]
pub(super) struct DrmVirtgpuResourceCreate {
    pub target: u32,
    pub format: u32,
    pub bind: u32,
    pub width: u32,
    pub height: u32,
    pub depth: u32,
    pub array_size: u32,
    pub last_level: u32,
    pub nr_samples: u32,
    pub flags: u32,
    pub bo_handle: u32,
    pub res_handle: u32,
    pub size: u32,
    pub stride: u32,
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, Pod)]
pub(super) struct DrmVirtgpuResourceInfo {
    pub bo_handle: u32,
    pub res_handle: u32,
    pub size: u32,
    pub blob_mem: u32,
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, Pod)]
pub(super) struct DrmVirtgpu3dBox {
    pub x: u32,
    pub y: u32,
    pub z: u32,
    pub w: u32,
    pub h: u32,
    pub d: u32,
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, Pod)]
pub(super) struct DrmVirtgpu3dTransferToHost {
    pub bo_handle: u32,
    pub box_: DrmVirtgpu3dBox,
    pub level: u32,
    pub offset: u32,
    pub stride: u32,
    pub layer_stride: u32,
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, Pod)]
pub(super) struct DrmVirtgpu3dTransferFromHost {
    pub bo_handle: u32,
    pub box_: DrmVirtgpu3dBox,
    pub level: u32,
    pub offset: u32,
    pub stride: u32,
    pub layer_stride: u32,
}

pub(super) const VIRTGPU_BLOB_MEM_GUEST: u32 = 0x0001;
pub(super) const VIRTGPU_BLOB_MEM_HOST3D: u32 = 0x0002;
pub(super) const VIRTGPU_BLOB_MEM_HOST3D_GUEST: u32 = 0x0003;

bitflags::bitflags! {
    pub(super) struct VirtioGpuBlobFlags: u32 {
        const USE_MAPPABLE = 1 << 0;
        const USE_SHAREABLE = 1 << 1;
        const USE_CROSS_DEVICE = 1 << 2;
    }
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, Pod)]
pub(super) struct DrmVirtgpuResourceCreateBlob {
    pub blob_mem: u32,
    pub blob_flags: u32,
    pub bo_handle: u32,
    pub res_handle: u32,
    pub size: u64,
    pub pad: u32,
    pub cmd_size: u32,
    pub cmd: u64,
    pub blob_id: u64,
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, Pod)]
pub(super) struct DrmVirtgpu3dWait {
    pub handle: u32,
    pub flags: u32,
}

bitflags::bitflags! {
    pub(super) struct VirtioGpuWaitFlags: u32 {
        const NOWAIT = 1;
    }
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, Pod)]
pub(super) struct DrmVirtgpuGetCaps {
    pub cap_set_id: u32,
    pub cap_set_ver: u32,
    pub addr: u64,
    pub size: u32,
    pub pad: u32,
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, Pod)]
pub(super) struct DrmVirtgpuContextSetParam {
    pub param: u64,
    pub value: u64,
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, Pod)]
pub(super) struct DrmVirtgpuContextInit {
    pub num_params: u32,
    pub pad: u32,
    pub ctx_set_params: u64,
}
