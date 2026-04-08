use alloc::sync::Arc;

use aster_gpu::drm::{fence::DrmFence, syncobj::DrmSyncobj};

use crate::{
    events::IoEvents,
    fs::{
        inode_handle::FileIo,
        utils::{InodeIo, StatusFlags},
    },
    prelude::*,
    process::signal::{PollHandle, Pollable},
};

#[derive(Debug)]
pub struct DrmSyncobjFd {
    syncobj: Arc<DrmSyncobj>
}

impl DrmSyncobjFd {
    pub fn new(fence: Arc<dyn DrmFence>) -> Self {
        todo!()
    }
    pub fn fence(&self) -> Arc<dyn DrmFence> {
        todo!()
    }
}

impl InodeIo for DrmSyncobjFd {
    fn read_at(
        &self,
        offset: usize,
        writer: &mut VmWriter,
        status_flags: StatusFlags,
    ) -> Result<usize> {
        todo!()
    }

    fn write_at(
        &self,
        offset: usize,
        reader: &mut VmReader,
        status_flags: StatusFlags,
    ) -> Result<usize> {
        todo!()
    }
}

impl Pollable for DrmSyncobjFd {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        todo!()
    }
}

impl FileIo for DrmSyncobjFd {
    fn check_seekable(&self) -> Result<()> {
        todo!()
    }

    fn is_offset_aware(&self) -> bool {
        todo!()
    }
}



#[derive(Debug)]
pub struct DrmSyncfileFd {
    fence: Arc<dyn DrmFence>,
}

impl DrmSyncfileFd {
    pub fn new(fence: Arc<dyn DrmFence>) -> Self {
        todo!()
    }
    pub fn fence(&self) -> Arc<dyn DrmFence> {
        todo!()
    }
}

impl InodeIo for DrmSyncfileFd {
    fn read_at(
        &self,
        offset: usize,
        writer: &mut VmWriter,
        status_flags: StatusFlags,
    ) -> Result<usize> {
        todo!()
    }

    fn write_at(
        &self,
        offset: usize,
        reader: &mut VmReader,
        status_flags: StatusFlags,
    ) -> Result<usize> {
        todo!()
    }
}

impl Pollable for DrmSyncfileFd {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        todo!()
    }
}

impl FileIo for DrmSyncfileFd {
    fn check_seekable(&self) -> Result<()> {
        todo!()
    }

    fn is_offset_aware(&self) -> bool {
        todo!()
    }
}
