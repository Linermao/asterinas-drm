use alloc::sync::Arc;
use core::{any::Any, fmt::Debug};

use crate::drm::{device::DrmDevice, gem::DrmGemObject};

bitflags::bitflags! {
    pub struct DrmDriverFeatures: u32 {
        const GEM              = 1 << 0;
        const MODESET          = 1 << 1;
        const RENDER           = 1 << 3;
        const ATOMIC           = 1 << 4;
        const SYNCOBJ          = 1 << 5;
        const SYNCOBJ_TIMELINE = 1 << 6;
        const COMPUTE_ACCEL    = 1 << 7;
        const GEM_GPUVA        = 1 << 8;
        const CURSOR_HOTSPOT   = 1 << 9;

        const USE_AGP          = 1 << 25;
        const LEGACY           = 1 << 26;
        const PCI_DMA          = 1 << 27;
        const SG               = 1 << 28;
        const HAVE_DMA         = 1 << 29;
        const HAVE_IRQ         = 1 << 30;
    }
}

/// Defines the interface implemented by a concrete DRM GPU driver.
///
/// `DrmDriver` represents the **driver-level logic** for a specific class of
/// GPU devices. It is responsible for device instantiation, feature
/// declaration, and handling operations that are part of
/// the generic DRM core.
///
/// A single `DrmDriver` instance may manage multiple DRM devices (e.g. multiple
/// GPUs of the same type), each identified by a unique index.
pub trait DrmDriver: Send + Sync + Any + Debug {
    /// Device name, description and date (for debugging / identification).
    fn name(&self) -> &str;
    fn desc(&self) -> &str;
    fn date(&self) -> &str;

    /// Create and initialize a DRM device instance managed by this driver.
    ///
    /// This is typically called by the DRM core during probing after a
    /// compatible GPU device has been matched to this driver.
    fn create_device(&self, index: u32) -> Result<Arc<DrmDevice>, ()>;

    /// Returns the feature flags supported by devices driven by this driver.
    ///
    /// The DRM core uses this information to enable or restrict generic
    /// functionality (e.g. modesetting, GEM, render node support).
    fn driver_features(&self) -> DrmDriverFeatures;

    /// Handle device-specific command / ioctl.
    fn handle_command(&self, _cmd: u32, _data: usize) -> Result<(), ()> {
        Ok(())
    }

    fn driver_ops(&self) -> DrmDriverOps;
}

/// Defines and registers a DRM driver with the global driver table.
///
/// This macro generates:
/// - A concrete, zero-sized DRM driver type.
/// - A `register_driver()` helper function that inserts the driver instance
///   into the DRM driver table under a given name.
///
/// TODO: Do not rely on device.name() for driver matching.
#[macro_export]
macro_rules! drm_register_driver {
    (
        $name:ident,
        $drv_name:expr
    ) => {
        #[derive(Debug)]
        pub struct $name {}

        // pub fn register_driver(driver_table: &mut $crate::drm::DriverTable) {
        //     driver_table.insert($drv_name.to_string(), alloc::sync::Arc::new($name {}));
        // }
    };
}

/// Optional driver operations for generic DRM capabilities.
///
/// This struct defines a set of driver-provided callbacks that the DRM core
/// may invoke for standard buffer management operations. Drivers that support
/// these features (e.g., KMS dumb buffers) should supply appropriate
/// function implementations; drivers that do not can leave the fields `None`.
///
/// Linux DRM exposes similar optional hooks (such as `dumb_create` and
/// `dumb_map_offset`) that are invoked via the corresponding ioctls when
/// userspace requests simple scanout buffer creation or mmap offsets.
pub struct DrmDriverOps {
    /// This creates a new dumb buffer in the driver's backing storage manager (GEM,
    /// TTM or something else entirely) and returns the resulting buffer handle. This
    /// handle can then be wrapped up into a framebuffer modeset object.
    pub dumb_create: Option<DumbCreateProvider>,
}

impl DrmDriverOps {
    pub const EMPTY: Self = Self { dumb_create: None };

    pub fn merge(self, other: Self) -> Self {
        Self {
            dumb_create: if other.dumb_create.is_some() {
                other.dumb_create
            } else {
                self.dumb_create
            },
        }
    }
}

pub enum DumbCreateProvider {
    Memfd,
    Custom(fn(width: u32, height: u32, bpp: u32) -> Result<Arc<DrmGemObject>, ()>),
}

/// This macro recursively merges a list of `DrmDriverOps` expressions into
/// one. Each item in the invocation is merged with the next by calling
/// `.merge(...)` on the head and the result of the recursive call on the
/// tail, yielding a consolidated `DrmDriverOps`.
///
/// produces a merged `DrmDriverOps` that combines the supplied ops with
/// the baseline `DrmDriverOps::EMPTY`.
///
/// This pattern uses declarative macro recursion to build up the final ops
/// set at compile time.
///
/// Example:
/// ```rust
/// fn driver_ops(&self) -> DrmDriverOps {
///     drm_driver_ops!(DRM_MEMFD_DRIVER_OPS, ..DrmDriverOps::EMPTY)
/// }
/// ```
#[macro_export]
macro_rules! drm_driver_ops {
    (..$base:expr) => {
        $base
    };

    ($head:expr, $($tail:tt)+) => {
        $head.merge(drm_driver_ops!($($tail)+))
    };
}
