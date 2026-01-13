use crate::{
    device::drm::{DrmDriver, DrmMinor},
    events::IoEvents,
    fs::{
        file_handle::Mappable,
        inode_handle::FileIo,
        utils::{InodeIo, StatusFlags},
    },
    prelude::*,
    process::signal::{PollHandle, Pollable},
    util::ioctl::RawIoctl,
};

/// Represents an open DRM file descriptor exposed to userspace.
///
/// `DrmFile` is created on each successful `open()` of a DRM device node
/// (e.g. `/dev/dri/cardX`, `/dev/dri/renderDX`). It serves as the **per-open
/// execution context** for all userspace interactions with the DRM subsystem.
///
/// Responsibilities:
/// - Dispatching ioctl requests issued from userspace.
/// - Enforcing access restrictions and semantics defined by the associated
///   DRM minor (primary, render, control, etc.).
///
/// `DrmFile` does not own device-wide state. Instead, it holds a reference to
/// the `DrmMinor` through which it was opened, and all operations are ultimately
/// routed to the underlying `DrmDevice` shared by all minors of the same GPU.
///
/// Each `DrmFile` instance is independent and represents a single userspace
/// file descriptor, while the underlying DRM device and driver state are
/// shared across all open files.
#[derive(Debug)]
pub(super) struct DrmFile<D: DrmDriver> {
    device: Arc<DrmMinor<D>>,
}

impl<D: DrmDriver> Pollable for DrmFile<D> {
    fn poll(&self, mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        let events = IoEvents::IN | IoEvents::OUT;
        events & mask
    }
}

impl<D: DrmDriver> DrmFile<D> {
    // Additional methods can be added here if needed
    pub fn new(device: Arc<DrmMinor<D>>) -> Self {
        Self { device }
    }
}

impl<D: DrmDriver> InodeIo for DrmFile<D> {
    fn read_at(
        &self,
        _offset: usize,
        _writer: &mut VmWriter,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "drm: read not supported");
    }

    fn write_at(
        &self,
        _offset: usize,
        _reader: &mut VmReader,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "drm: write not supported");
    }
}

impl<D: DrmDriver> FileIo for DrmFile<D> {
    fn check_seekable(&self) -> Result<()> {
        Ok(())
    }

    fn is_offset_aware(&self) -> bool {
        true
    }

    fn mappable(&self) -> Result<Mappable> {
        todo!()
    }

    fn ioctl(&self, _raw_ioctl: RawIoctl) -> Result<i32> {
        todo!()
    }
}
