bitflags::bitflags! {
    pub struct VirtGpuExecBufferFlags: u32 {
        const FENCE_FD_IN = 0x01;
        const FENCE_FD_OUT = 0x02;
        const RING_IDX = 0x04;
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
struct VirtGpuExecBuffer {
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

// TODO: special device ioctl
// drm_ioc!(DrmIoctlVirtGpuExecBuffer,     DRM_IOCTL_VIRTGPU_EXECBUFFER,       b'd', 0x42, InOutData<VirtGpuExecBuffer>,
//     DrmIoctlFlags::ANY);
pub(super) const DRM_IOCTL_VIRTGPU_EXECBUFFER: u32 = 0xC0406442;