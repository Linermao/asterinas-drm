// SPDX-License-Identifier: MPL-2.0

//! PRIME handle and dma-buf descriptor conversion ioctls.

use aster_core::{
    fs::dma_buf::{self, DmaBufFlags},
    prelude::*,
};

use crate::{file::DrmFile, ioctl::ioctl_defs};

impl DrmFile {
    pub(super) fn drm_prime_handle_to_fd(&self, cmd: ioctl_defs::PrimeHandleToFd) -> Result<i32> {
        let mut args = cmd.read()?;
        let flags = DmaBufFlags::from_bits(args.flags).ok_or_else(|| {
            Error::with_message(Errno::EINVAL, "the PRIME export flags are invalid")
        })?;
        args.fd = self.export_prime_buffer(args.handle, flags)?;
        cmd.write(&args)?;
        Ok(0)
    }

    pub(super) fn drm_prime_fd_to_handle(&self, cmd: ioctl_defs::PrimeFdToHandle) -> Result<i32> {
        let mut args = cmd.read()?;
        let buffer = dma_buf::get(args.fd)?;
        // Linux ignores the flags field on import.
        self.import_prime_buffer(buffer.as_ref(), |handle| {
            args.handle = handle;
            cmd.write(&args)
        })?;
        Ok(0)
    }
}

/// `struct drm_prime_handle` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/drm/drm.h>.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(super) struct DrmPrimeHandle {
    handle: u32,
    flags: u32,
    fd: i32,
}
