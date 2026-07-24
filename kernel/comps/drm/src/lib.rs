// SPDX-License-Identifier: MPL-2.0

#![no_std]
#![deny(unsafe_code)]

extern crate alloc;
#[macro_use]
extern crate ostd_pod;

// Set this crate's log prefix for `ostd::log`.
macro_rules! __log_prefix {
    () => {
        "drm: "
    };
}

mod atomic;
mod device;
mod event;
mod gem;
mod geometry;
mod kms;
mod simpledrm;
mod sync;

use alloc::{sync::Arc, vec::Vec};

use aster_framebuffer::FRAMEBUFFER;
pub use atomic::{DrmAtomicFlags, DrmAtomicObjectRequest, DrmAtomicOps};
use component::{ComponentInitError, init_component};
pub use device::{
    DrmDevice, DrmDeviceBusInfo, DrmDeviceCaps, DrmDevicePrivate, DrmFeatures, DrmIoctlCommandCtx,
};
pub use event::DrmIoctlEventCtx;
pub use gem::{
    DrmGemOps, DrmIoctlGemCtx, DrmSgEntry,
    object::{DrmGemMapPage, DrmGemObject},
    vma_manager::{DrmVmaOffsetManager, DrmVmaOffsetNode},
};
pub use geometry::DrmRect;
pub use kms::{
    DrmKmsOps,
    display::{DrmDisplayFormat, DrmDisplayInfo, DrmDisplayMode, DrmModeModeInfo, SubpixelOrder},
    edid::DrmEdid,
    object::{
        DrmKmsObject, DrmKmsObjectStore, DrmKmsObjectType, KmsObjectId,
        builder::DrmKmsObjectBuilder,
        connector::{DrmConnState, DrmConnStatus, DrmConnType, DrmConnector, DrmConnectorSnapshot},
        crtc::{DrmCrtc, DrmCrtcSnapshot, DrmCrtcState},
        encoder::{DrmEncoder, DrmEncoderState, DrmEncoderType},
        framebuffer::{DRM_FORMAT_MAX_PLANES, DrmFramebuffer},
        plane::{DrmPlane, DrmPlaneState, DrmPlaneType},
        property::{
            DRM_PROP_NAME_LEN, DrmKmsObjectProp, DrmProperty, DrmPropertyEnum, DrmPropertyFlags,
            DrmPropertyKind, DrmPropertySpec, blob::DrmPropertyBlob,
        },
    },
    vblank::DrmPendingVblankEvent,
};
use ostd::sync::Mutex;
use spin::Once;
pub use sync::{
    DrmFence, DrmFenceCallback, DrmFenceStatus, DrmSyncObj, DrmSyncObjCreateFlags,
    DrmSyncObjQueryFlags, DrmSyncObjWaitCondition, DrmSyncObjWaitFlags,
};

use crate::simpledrm::SimpleDrmDevice;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DrmError {
    /// Generic invalid argument or state
    Invalid,
    /// Object not found (CRTC / FB / GEM handle / connector, etc.)
    NotFound,
    /// Operation not supported by this driver / device
    NotSupported,
    /// Operation is recognized but not implemented by this driver / device.
    FunctionNotImplemented,
    /// Resource temporarily unavailable (busy, in use)
    Busy,
    /// Permission or access violation
    PermissionDenied,
    /// Bad userspace address
    BadAddress,
    /// Memory allocation or mapping failure
    NoMemory,
    /// Resource already exist.
    AlreadyExist,
    /// Ioctl not found.
    IoctlNotFound,
}

impl From<DrmError> for ComponentInitError {
    fn from(error: DrmError) -> Self {
        match error {
            DrmError::AlreadyExist => {
                ostd::warn!("The device already registered")
            }
            _ => {}
        }
        ComponentInitError::Unknown
    }
}

pub fn register_drm_device(device: Arc<dyn DrmDevice>) -> Result<(), DrmError> {
    let component = COMPONENT
        .get()
        .expect("aster-drm component not initialized");

    component.drm_devices.lock().push(device);

    Ok(())
}

pub fn registered_drm_devices() -> Vec<Arc<dyn DrmDevice>> {
    let component = COMPONENT
        .get()
        .expect("aster-drm component not initialized");

    component.drm_devices.lock().clone()
}

pub fn unregister_drm_device(device: &Arc<dyn DrmDevice>) -> Result<Arc<dyn DrmDevice>, DrmError> {
    let component = COMPONENT
        .get()
        .expect("aster-drm component not initialized");

    let mut devices = component.drm_devices.lock();
    if let Some(pos) = devices.iter().position(|d| Arc::ptr_eq(d, device)) {
        Ok(devices.remove(pos))
    } else {
        Err(DrmError::NotFound)
    }
}

static COMPONENT: Once<Component> = Once::new();

#[init_component]
fn component_init() -> Result<(), ComponentInitError> {
    let component = Component::init()?;
    COMPONENT.call_once(|| component);

    if FRAMEBUFFER.get().is_some() {
        match SimpleDrmDevice::new() {
            Ok(device) => register_drm_device(Arc::new(device))?,
            Err(error) => {
                ostd::warn!("[kernel] DRM: failed to initialize simpledrm: {:?}", error);
            }
        }
    }

    Ok(())
}

#[derive(Debug)]
struct Component {
    drm_devices: Mutex<Vec<Arc<dyn DrmDevice>>>,
}

impl Component {
    fn init() -> Result<Self, ComponentInitError> {
        Ok(Self {
            drm_devices: Mutex::new(Vec::new()),
        })
    }
}
