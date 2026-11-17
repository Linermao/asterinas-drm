// SPDX-License-Identifier: MPL-2.0

//! Minimal dma-buf files for sharing CPU-coherent buffers.
//!
//! Exporters retain their backing storage through [`DmaBuf`]. Files can be
//! duplicated or passed over UNIX sockets, mapped with shared semantics, and
//! imported through [`get`]. This interface has no device DMA attachments or
//! asynchronous fences; exporters must use CPU-coherent storage and complete
//! all accesses synchronously.

use core::fmt::{self, Display};

use ostd::{mm::PAGE_SIZE, task::Task};

use crate::{
    dispatch_ioctl,
    events::IoEvents,
    fs::{
        file::{
            AccessMode, CreationFlags, FileCommon, FileLike, Mappable, MappableObject, MappedObject,
            SeekFrom, StatusFlags,
            file_table::{FdFlags, FileTableInsert},
        },
        pseudofs::AnonInodeFs,
    },
    prelude::*,
    process::{
        posix_thread::AsThreadLocal,
        signal::{PollHandle, Pollable},
    },
    util::ioctl::RawIoctl,
    vm::vmar::{FileMmapRequest, MapHandle},
};

/// A shareable CPU-coherent buffer with no outstanding asynchronous accesses.
///
/// A mapped object returned by [`Mappable::map`] must retain the backing storage
/// independently of the buffer and its exported files.
pub trait DmaBuf: Mappable + Any + Debug + Send + Sync {
    /// Returns the fixed, nonzero, page-aligned buffer size in bytes.
    fn size(&self) -> usize;
}

impl dyn DmaBuf {
    /// Returns the exporter-specific buffer, if its type matches.
    pub fn downcast_ref<T: DmaBuf>(&self) -> Option<&T> {
        (self as &dyn Any).downcast_ref()
    }
}

bitflags! {
    /// File creation flags accepted by DRM PRIME exports.
    #[derive(Clone, Copy, Debug)]
    pub struct DmaBufFlags: u32 {
        /// Closes the descriptor on exec.
        const CLOEXEC = CreationFlags::O_CLOEXEC.bits();
        /// Allows writable shared mappings; otherwise the file is read-only.
        const RDWR = AccessMode::O_RDWR as u32;
    }
}

/// Installs a dma-buf file in the current process and returns its descriptor.
///
/// The installed file owns a reference to `buffer` independently of the exporter.
/// As with Linux PRIME exports, installation precedes copying the descriptor to
/// userspace; callers must not close a published descriptor on a copy-out error,
/// since another thread may already have closed and reused it.
pub fn export(buffer: Arc<dyn DmaBuf>, flags: DmaBufFlags) -> Result<i32> {
    let size = buffer.size();
    if size == 0 || !size.is_multiple_of(PAGE_SIZE) || size > isize::MAX as usize {
        return_errno_with_message!(Errno::EINVAL, "the dma-buf size is invalid");
    }

    let access_mode = if flags.contains(DmaBufFlags::RDWR) {
        AccessMode::O_RDWR
    } else {
        AccessMode::O_RDONLY
    };
    let file = Arc::new(DmaBufFile {
        buffer,
        size,
        common: FileCommon::new(
            AnonInodeFs::new_path(|_| "anon_inode:[dmabuf]".to_string()),
            access_mode,
            StatusFlags::empty(),
        ),
    });
    let fd_flags = if flags.contains(DmaBufFlags::CLOEXEC) {
        FdFlags::CLOEXEC
    } else {
        FdFlags::empty()
    };

    let task = Task::current().unwrap();
    let thread_local = task.as_thread_local().unwrap();
    let file_table = thread_local.borrow_file_table();
    let fd = file_table.unwrap().lock_and_insert(file, fd_flags)?;
    Ok(fd.into())
}

/// Gets an independently owned buffer from a dma-buf descriptor in the current process.
///
/// Returns `EBADF` for an invalid descriptor and `EINVAL` for another file type.
pub fn get(raw_fd: i32) -> Result<Arc<dyn DmaBuf>> {
    let task = Task::current().unwrap();
    let thread_local = task.as_thread_local().unwrap();
    let file = {
        let file_table = thread_local.borrow_file_table();
        let file = file_table
            .unwrap()
            .read()
            .get_file(raw_fd.try_into()?)?
            .clone();
        file
    };

    let file = file.downcast_ref::<DmaBufFile>().ok_or_else(|| {
        Error::with_message(Errno::EINVAL, "the file is not a dma-buf")
    })?;
    Ok(file.buffer.clone())
}

struct DmaBufFile {
    buffer: Arc<dyn DmaBuf>,
    size: usize,
    common: FileCommon,
}

impl FileLike for DmaBufFile {
    fn common(&self) -> &FileCommon {
        &self.common
    }

    fn mappable(&self, request: FileMmapRequest) -> Result<MappableObject<'_>> {
        if !request.is_shared() {
            return_errno_with_message!(Errno::EINVAL, "private dma-buf mappings are not supported");
        }
        Ok(MappableObject::Device(self))
    }

    fn seek(&self, seek_from: SeekFrom) -> Result<usize> {
        // Reference: <https://elixir.bootlin.com/linux/v6.18/source/drivers/dma-buf/dma-buf.c>.
        // dma_buf_llseek() only allows size discovery and seeking to zero.
        match seek_from {
            SeekFrom::Start(0) => Ok(0),
            SeekFrom::End(0) => Ok(self.size),
            _ => return_errno_with_message!(Errno::EINVAL, "the dma-buf seek is invalid"),
        }
    }

    fn ioctl(&self, raw_ioctl: RawIoctl) -> Result<i32> {
        use ioctl_defs::Sync;

        dispatch_ioctl!(match raw_ioctl {
            cmd @ Sync => {
                let flags: u64 = cmd.read()?;
                const DMA_BUF_SYNC_READ: u64 = 1 << 0;
                const DMA_BUF_SYNC_WRITE: u64 = 1 << 1;
                const DMA_BUF_SYNC_END: u64 = 1 << 2;
                const ACCESS_MASK: u64 = DMA_BUF_SYNC_READ | DMA_BUF_SYNC_WRITE;
                const VALID_MASK: u64 = ACCESS_MASK | DMA_BUF_SYNC_END;
                if flags & !VALID_MASK != 0 || flags & ACCESS_MASK == 0 {
                    return_errno_with_message!(Errno::EINVAL, "the dma-buf sync flags are invalid");
                }
                // Exporters only provide synchronous CPU-coherent buffers, so
                // no DMA cache maintenance or fence wait is needed.
                Ok(0)
            }
            _ => return_errno_with_message!(Errno::ENOTTY, "the dma-buf ioctl is not supported"),
        })
    }

    fn dump_proc_fdinfo(self: Arc<Self>, fd_flags: FdFlags) -> Box<dyn Display> {
        struct FdInfo {
            file: Arc<DmaBufFile>,
            flags: FdFlags,
        }

        impl Display for FdInfo {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                let mut flags = self.file.common.status_flags().bits()
                    | self.file.common.access_mode() as u32;
                if self.flags.contains(FdFlags::CLOEXEC) {
                    flags |= CreationFlags::O_CLOEXEC.bits();
                }
                let path = self.file.common.path();
                writeln!(f, "pos:\t0")?;
                writeln!(f, "flags:\t0{:o}", flags)?;
                writeln!(f, "mnt_id:\t{}", path.mount_node().id())?;
                writeln!(f, "ino:\t{}", path.inode().ino())?;
                writeln!(f, "size:\t{}", self.file.size)
            }
        }

        Box::new(FdInfo {
            file: self,
            flags: fd_flags,
        })
    }
}

impl Mappable for DmaBufFile {
    fn map(&self, offset: usize, handle: MapHandle) -> Result<Box<dyn MappedObject>> {
        let size = handle.size();
        if !offset.is_multiple_of(PAGE_SIZE)
            || size == 0
            || offset.checked_add(size).is_none_or(|end| end > self.size)
        {
            return_errno_with_message!(Errno::EINVAL, "the dma-buf mapping exceeds the buffer");
        }
        self.buffer.map(offset, handle)
    }
}

impl Pollable for DmaBufFile {
    fn poll(&self, mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        // No asynchronous operations can hold these buffers busy.
        mask & (IoEvents::IN | IoEvents::OUT)
    }
}

mod ioctl_defs {
    use crate::{ioc, util::ioctl::InData};

    // `struct dma_buf_sync` contains one __u64 flags field.
    // Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/linux/dma-buf.h>.
    pub(super) type Sync = ioc!(DMA_BUF_IOCTL_SYNC, b'b', 0, InData<u64>);
}
