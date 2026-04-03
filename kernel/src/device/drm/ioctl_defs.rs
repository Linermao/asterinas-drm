use aster_gpu::drm::ioctl::*;

use crate::{
    device::drm::file::DrmFile, 
    prelude::*, 
    util::ioctl::{InData, InOutData, NoData}
};

#[derive(Debug)]
pub(crate) enum DrmIoctlFlags {
    Any,
}

impl DrmIoctlFlags {
    pub const ANY: Self = Self::Any;

    fn allows(self, _file: &DrmFile) -> bool {
        match self {
            Self::Any => true,
        }
    }
}

pub(crate) trait DrmIoctlFlagsInfo {
    const PERMISSION: DrmIoctlFlags;
}

pub(crate) fn check_drm_ioctl_flags<T: DrmIoctlFlagsInfo>(file: &DrmFile) -> Result<()> {
    if T::PERMISSION.allows(file) {
        Ok(())
    } else {
        return_errno!(Errno::EACCES);
    }
}

#[macro_export]
macro_rules! drm_ioc {
    ($name:ident, $linux_name:ident, $magic:expr, $nr:expr, $data:ty, $perm:expr) => {
        pub(super) type $name = $crate::util::ioctl::ioc!($linux_name, $magic, $nr, $data);

        impl $crate::device::drm::ioctl_defs::DrmIoctlFlagsInfo for $name {
            const PERMISSION: $crate::device::drm::ioctl_defs::DrmIoctlFlags = $perm;
        }
    };
}

#[macro_export]
macro_rules! drm_dispatch {
    ($file:expr, match $raw:ident {}) => {
        ()
    };

    ($file:expr, match $raw:ident { _ => $arm:expr $(,)? }) => {
        $arm
    };

    ($file:expr, match $raw:ident {
        $ty0:ty $(| $ty1:ty)* => $arm:block $(,)?
        $($rest:tt)*
    }) => {
        if <$ty0>::try_from_raw($raw).is_some() {
            $crate::device::drm::ioctl_defs::check_drm_ioctl_flags::<$ty0>($file)?;
            $arm
        } $( else if <$ty1>::try_from_raw($raw).is_some() {
            $crate::device::drm::ioctl_defs::check_drm_ioctl_flags::<$ty1>($file)?;
            $arm
        } )* else {
            $crate::drm_dispatch!($file, match $raw { $($rest)* })
        }
    };

    ($file:expr, match $raw:ident {
        $bind:ident @ $ty:ty => $arm:block $(,)?
        $($rest:tt)*
    }) => {
        if let Some($bind) = <$ty>::try_from_raw($raw) {
            $crate::device::drm::ioctl_defs::check_drm_ioctl_flags::<$ty>($file)?;
            $arm
        } else {
            $crate::drm_dispatch!($file, match $raw { $($rest)* })
        }
    };
}

#[macro_export]
macro_rules! dispatch_drm_ioctl {
    ($file:expr, $($tt:tt)*) => {
        $crate::drm_dispatch!($file, $($tt)*)
    };
}

drm_ioc!(DrmIoctlVersion,               DRM_IOCTL_VERSION,                  b'd', 0x00, InOutData<DrmVersion>, 
    DrmIoctlFlags::ANY);
drm_ioc!(DrmIoctlGetCap,                DRM_IOCTL_GET_CAP,                  b'd', 0x0c, InOutData<DrmGetCap>, 
    DrmIoctlFlags::ANY);
drm_ioc!(DrmIoctlSetClientCap,          DRM_IOCTL_SET_CLIENT_CAP,           b'd', 0x0d, InData<DrmSetClientCap>, 
    DrmIoctlFlags::ANY);
drm_ioc!(DrmIoctlSetMaster,             DRM_IOCTL_SET_MASTER,               b'd', 0x1e, NoData, 
    DrmIoctlFlags::ANY);
drm_ioc!(DrmIoctlDropMaster,            DRM_IOCTL_DROP_MASTER,              b'd', 0x1f, NoData, 
    DrmIoctlFlags::ANY);
drm_ioc!(DrmIoctlModeGetResources,      DRM_IOCTL_MODE_GETRESOURCES,        b'd', 0xa0, InOutData<DrmModeGetResources>, 
    DrmIoctlFlags::ANY);
drm_ioc!(DrmIoctlModeGetCrtc,           DRM_IOCTL_MODE_GETCRTC,             b'd', 0xa1, InOutData<DrmModeCrtc>, 
    DrmIoctlFlags::ANY);
drm_ioc!(DrmIoctlModeSetCrtc,           DRM_IOCTL_MODE_SETCRTC,             b'd', 0xa2, InOutData<DrmModeCrtc>, 
    DrmIoctlFlags::ANY);
drm_ioc!(DrmIoctlModeCursor,            DRM_IOCTL_MODE_CURSOR,              b'd', 0xa3, InOutData<DrmModeCursor>, 
    DrmIoctlFlags::ANY);
drm_ioc!(DrmIoctlModeGetEncoder,        DRM_IOCTL_MODE_GETENCODER,          b'd', 0xa6, InOutData<DrmModeGetEncoder>, 
    DrmIoctlFlags::ANY);
drm_ioc!(DrmIoctlModeGetConnector,      DRM_IOCTL_MODE_GETCONNECTOR,        b'd', 0xa7, InOutData<DrmModeGetConnector>, 
    DrmIoctlFlags::ANY);
drm_ioc!(DrmIoctlModeGetProperty,       DRM_IOCTL_MODE_GETPROPERTY,         b'd', 0xaa, InOutData<DrmModeGetProperty>, 
    DrmIoctlFlags::ANY);
drm_ioc!(DrmIoctlModeGetPropBlob,       DRM_IOCTL_MODE_GETPROPBLOB,         b'd', 0xac, InOutData<DrmModeGetBlob>,
    DrmIoctlFlags::ANY);
drm_ioc!(DrmIoctlModeAddFB,             DRM_IOCTL_MODE_ADDFB,               b'd', 0xae, InOutData<DrmModeFbCmd>,
    DrmIoctlFlags::ANY);
drm_ioc!(DrmIoctlModeRmFB,              DRM_IOCTL_MODE_RMFB,                b'd', 0xaf, InOutData<u32>,
    DrmIoctlFlags::ANY);
drm_ioc!(DrmIoctlModePageFlip,          DRM_IOCTL_MODE_PAGE_FLIP,           b'd', 0xb0, InOutData<DrmModeCrtcPageFlip>,
    DrmIoctlFlags::ANY);
drm_ioc!(DrmIoctlModeDirtyFb,           DRM_IOCTL_MODE_DIRTYFB,             b'd', 0xb1, InOutData<DrmModeFbDirtyCmd>,
    DrmIoctlFlags::ANY);
drm_ioc!(DrmIoctlModeCreateDumb,        DRM_IOCTL_MODE_CREATE_DUMB,         b'd', 0xb2, InOutData<DrmModeCreateDumb>,
    DrmIoctlFlags::ANY);
drm_ioc!(DrmIoctlModeMapDumb,           DRM_IOCTL_MODE_MAP_DUMB,            b'd', 0xb3, InOutData<DrmModeMapDumb>,
    DrmIoctlFlags::ANY);
drm_ioc!(DrmIoctlModeDestroyDumb,       DRM_IOCTL_MODE_DESTROY_DUMB,        b'd', 0xb4, InOutData<DrmModeDestroyDumb>,
    DrmIoctlFlags::ANY);
drm_ioc!(DrmIoctlModeGetPlaneResources, DRM_IOCTL_MODE_GETPLANERESOURCES,   b'd', 0xb5, InOutData<DrmModeGetPlaneRes>,
    DrmIoctlFlags::ANY);
drm_ioc!(DrmIoctlModeGetPlane,          DRM_IOCTL_MODE_GETPLANE,            b'd', 0xb6, InOutData<DrmModeGetPlane>,
    DrmIoctlFlags::ANY);
drm_ioc!(DrmIoctlModeAddFB2,            DRM_IOCTL_MODE_ADDFB2,              b'd', 0xb8, InOutData<DrmModeFbCmd2>,
    DrmIoctlFlags::ANY);
drm_ioc!(DrmIoctlModeObjectGetProps,    DRM_IOCTL_MODE_OBJ_GETPROPERTIES,   b'd', 0xb9, InOutData<DrmModeObjectGetProps>,
    DrmIoctlFlags::ANY);
drm_ioc!(DrmIoctlModeCursor2,           DRM_IOCTL_MODE_CURSOR2,             b'd', 0xbb, InOutData<DrmModeCursor2>, 
    DrmIoctlFlags::ANY);
drm_ioc!(DrmIoctlModeAtomic,            DRM_IOCTL_MODE_ATOMIC,              b'd', 0xbc, InOutData<DrmModeAtomic>, 
    DrmIoctlFlags::ANY);
drm_ioc!(DrmIoctlModeCreatePropBlob,    DRM_IOCTL_MODE_CREATEPROPBLOB,      b'd', 0xbd, InOutData<DrmModeCreateBlob>,
    DrmIoctlFlags::ANY);
drm_ioc!(DrmIoctlModeDestroyPropBlob,   DRM_IOCTL_MODE_DESTROYPROPBLOB,     b'd', 0xbe, InOutData<DrmModeDestroyBlob>,
    DrmIoctlFlags::ANY);
drm_ioc!(DrmIoctlSyncobjCreate,         DRM_IOCTL_SYNCOBJ_CREATE,           b'd', 0xbf, InOutData<DrmSyncobjCreate>,
    DrmIoctlFlags::ANY);
drm_ioc!(DrmIoctlSyncobjDestroy,        DRM_IOCTL_SYNCOBJ_DESTROY,          b'd', 0xc0, InOutData<DrmSyncobjDestroy>,
    DrmIoctlFlags::ANY);
drm_ioc!(DrmIoctlSyncobjWait,           DRM_IOCTL_SYNCOBJ_WAIT,             b'd', 0xc3, InOutData<DrmSyncobjWait>,
    DrmIoctlFlags::ANY);
drm_ioc!(DrmIoctlSyncobjReset,          DRM_IOCTL_SYNCOBJ_RESET,            b'd', 0xc4, InOutData<DrmSyncobjArray>,
    DrmIoctlFlags::ANY);
drm_ioc!(DrmIoctlSyncobjSignal,         DRM_IOCTL_SYNCOBJ_SIGNAL,           b'd', 0xc5, InOutData<DrmSyncobjArray>,
    DrmIoctlFlags::ANY);
// TODO: special device ioctl
drm_ioc!(DrmIoctlVirtGpuExecBuffer,     DRM_IOCTL_VIRTGPU_EXECBUFFER,       b'd', 0x42, InOutData<VirtGpuExecBuffer>,
    DrmIoctlFlags::ANY);