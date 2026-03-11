use aster_gpu::drm::ioctl::*;

use crate::{device::drm::file::DrmFile, prelude::*, util::ioctl::{InData, InOutData}};

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

drm_ioc!(DrmIoctlVersion,            DRM_IOCTL_VERSION,                b'd', 0x00, InOutData<DrmVersion>, 
    DrmIoctlFlags::ANY);
drm_ioc!(DrmIoctlModeGetResources,   DRM_IOCTL_MODE_GETRESOURCES,      b'd', 0xa0, InOutData<DrmModeGetResources>, 
    DrmIoctlFlags::ANY);
drm_ioc!(DrmIoctlModeGetCrtc,        DRM_IOCTL_MODE_GETCRTC,           b'd', 0xa1, InOutData<DrmModeCrtc>, 
    DrmIoctlFlags::ANY);
drm_ioc!(DrmIoctlModeSetCrtc,        DRM_IOCTL_MODE_SETCRTC,           b'd', 0xa2, InOutData<DrmModeCrtc>, 
    DrmIoctlFlags::ANY);
drm_ioc!(DrmIoctlModeGetEncoder,     DRM_IOCTL_MODE_GETENCODER,        b'd', 0xa6, InOutData<DrmModeGetEncoder>, 
    DrmIoctlFlags::ANY);
drm_ioc!(DrmIoctlModeGetConnector,   DRM_IOCTL_MODE_GETCONNECTOR,      b'd', 0xa7, InOutData<DrmModeGetConnector>, 
    DrmIoctlFlags::ANY);
drm_ioc!(DrmIoctlModeGetProperty,    DRM_IOCTL_MODE_GETPROPERTY,       b'd', 0xaa, InOutData<DrmModeGetProperty>, 
    DrmIoctlFlags::ANY);
drm_ioc!(DrmIoctlModeGetPropBlob,    DRM_IOCTL_MODE_GETPROPBLOB,       b'd', 0xac, InOutData<DrmModeGetBlob>,
    DrmIoctlFlags::ANY);
drm_ioc!(DrmIoctlModeAddFB,          DRM_IOCTL_MODE_ADDFB,             b'd', 0xae, InOutData<DrmModeFbCmd>,
    DrmIoctlFlags::ANY);
drm_ioc!(DrmIoctlModeRmFB,           DRM_IOCTL_MODE_RMFB,              b'd', 0xaf, InData<DrmModeFbCmd>,
    DrmIoctlFlags::ANY);
drm_ioc!(DrmIoctlModeCreateDumb,     DRM_IOCTL_MODE_CREATE_DUMB,       b'd', 0xb2, InOutData<DrmModeCreateDumb>,
    DrmIoctlFlags::ANY);
drm_ioc!(DrmIoctlModeMapDumb,        DRM_IOCTL_MODE_MAP_DUMB,          b'd', 0xb3, InOutData<DrmModeMapDumb>,
    DrmIoctlFlags::ANY);
drm_ioc!(DrmIoctlModeDestroyDumb,    DRM_IOCTL_MODE_DESTROY_DUMB,      b'd', 0xb4, InData<DrmModeDestroyDumb>,
    DrmIoctlFlags::ANY);
drm_ioc!(DrmIoctlModeObjectGetProps, DRM_IOCTL_MODE_OBJ_GETPROPERTIES, b'd', 0xb9, InOutData<DrmModeObjectGetProps>,
    DrmIoctlFlags::ANY);
drm_ioc!(DrmIoctlModeCreatePropBlob, DRM_IOCTL_MODE_CREATEPROPBLOB,    b'd', 0xbd, InOutData<DrmModeCreateBlob>,
    DrmIoctlFlags::ANY);
    

