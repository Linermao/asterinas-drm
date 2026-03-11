use core::{any::Any, fmt::Debug};

use alloc::sync::Arc;
use ostd::mm::{VmReader, VmWriter};

use crate::drm::DrmError;

pub trait DrmGemObject: Debug + Any + Sync + Send {
    fn pitch(&self) -> u32;
    fn size(&self) -> u64;
    fn read(&self, offset: usize, writer: &mut VmWriter) -> Result<usize, DrmError>;
    fn write(&self, offset: usize, reader: &mut VmReader) -> Result<usize, DrmError>;
    fn release(&self) -> Result<(), DrmError>;
}

impl dyn DrmGemObject {
    pub fn downcast_ref<T: DrmGemObject>(&self) -> Option<&T> {
        (self as &dyn Any).downcast_ref::<T>()
    }
}

pub trait DrmGemMemfd: Debug + Sync + Send {
    fn create_dumb(&self, name: &str, size: u64) -> Result<Arc<dyn DrmGemObject>, DrmError>;
}