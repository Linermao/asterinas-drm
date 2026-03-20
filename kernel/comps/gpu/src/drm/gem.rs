use alloc::{boxed::Box, sync::Arc, vec::Vec};
use core::{any::Any, fmt::Debug};

use ostd::mm::{VmReader, VmWriter};

use crate::drm::{DrmError, MemfdAllocatorType, ioctl::DrmModeCreateDumb};

#[derive(Debug)]
pub struct DrmSgEntry {
    pub addr: u64,
    pub len: u32,
}

#[derive(Debug)]
pub struct DrmSgTable {
    pub entries: Vec<DrmSgEntry>,
}

pub trait DrmGemObject: Debug + Any + Sync + Send {
    fn backend(&self) -> &Box<dyn DrmGemBackend>;
    fn pitch(&self) -> u32;
    fn size(&self) -> u64;
}

impl dyn DrmGemObject {
    pub fn read(&self, offset: usize, writer: &mut VmWriter) -> Result<usize, DrmError> {
        self.backend().read(offset, writer)
    }

    pub fn write(&self, offset: usize, reader: &mut VmReader) -> Result<usize, DrmError> {
        self.backend().write(offset, reader)
    }

    pub fn release(&self) -> Result<(), DrmError> {
        self.backend().release()
    }
}

pub trait DrmGemBackend: Debug + Any + Sync + Send {
    fn read(&self, offset: usize, writer: &mut VmWriter) -> Result<usize, DrmError>;
    fn write(&self, offset: usize, reader: &mut VmReader) -> Result<usize, DrmError>;
    fn release(&self) -> Result<(), DrmError>;
    fn get_pages_sgt(&self) -> Result<DrmSgTable, DrmError> {
        Err(DrmError::NotSupported)
    }
}

impl dyn DrmGemBackend {
    pub fn downcast_ref<T: DrmGemBackend>(&self) -> Option<&T> {
        (self as &dyn Any).downcast_ref::<T>()
    }
}