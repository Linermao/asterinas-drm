use alloc::sync::Arc;
use ostd::sync::Mutex;

use crate::drm::fence::DrmFence;

bitflags::bitflags! {
    pub struct DrmSyncobjCreateFlags: u32 {
        const SIGNALED = 0x1;
    }
}

bitflags::bitflags! {
    pub struct DrmSyncobjFdToHandleFlags: u32 {
        const IMPORT_SYNC_FILE = 0x1;
        const TIMELINE = 0x2;
    }
}

bitflags::bitflags! {
    pub struct DrmSyncobjHandleToFdFlags: u32 {
        const EXPORT_SYNC_FILE = 0x1;
        const TIMELINE = 0x2;
    }
}

bitflags::bitflags! {
    pub struct DrmSyncobjWaitFlags: u32 {
        const WAIT_ALL = 0x1;
        const WAIT_FOR_SUBMIT = 0x2;
        const WAIT_AVAILABLE = 0x4;
        const WAIT_DEADLINE = 0x8;
    }
}

#[derive(Debug)]
pub struct DrmSyncobj {
    fence: Mutex<Arc<dyn DrmFence>>,
}

impl DrmSyncobj {
    pub fn new() -> Self {
        todo!()
    }
}
