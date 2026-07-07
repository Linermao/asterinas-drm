// SPDX-License-Identifier: MPL-2.0

use core::{fmt::Display, sync::atomic::Ordering, time::Duration};

use aster_drm::{DrmError, DrmFence, DrmSyncObj, DrmSyncObjCreateFlags, DrmSyncObjWaitFlags};
use ostd::{
    mm::VmIo,
    sync::Waiter,
    task::{Task, TaskOptions},
};

use crate::{
    device::drm::{file::DrmFile, ioctl::*},
    events::IoEvents,
    fs::{
        file::{
            AccessMode, CreationFlags, FileLike,
            file_table::{FdFlags, FileDesc, RawFileDesc},
        },
        pseudofs::AnonInodeFs,
        vfs::path::Path,
    },
    prelude::*,
    process::signal::{PollHandle, Pollable, Pollee},
    time::{clocks::MonotonicClock, timer::Timeout, wait::ManagedTimeout},
};

struct DrmSyncFile {
    fence: Arc<DrmFence>,
    pollee: Pollee,
    pseudo_path: Path,
}

impl DrmSyncFile {
    fn new(fence: Arc<DrmFence>) -> Self {
        fn sync_file_anon_inode_path(_: &dyn crate::fs::vfs::inode::Inode) -> String {
            "anon_inode:[sync_file]".to_string()
        }

        let pollee = Pollee::new();
        let notify_pollee = pollee.clone();
        let notify_fence = fence.clone();
        let _ = TaskOptions::new(move || {
            notify_fence.wait();
            notify_pollee.notify(IoEvents::IN | IoEvents::OUT | IoEvents::RDNORM);
        })
        .spawn();

        Self {
            fence,
            pollee,
            pseudo_path: AnonInodeFs::new_path(sync_file_anon_inode_path),
        }
    }

    fn fence(&self) -> Arc<DrmFence> {
        self.fence.clone()
    }
}

impl Pollable for DrmSyncFile {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.pollee.poll_with(mask, poller, || {
            if self.fence.is_signaled() {
                IoEvents::IN | IoEvents::OUT | IoEvents::RDNORM
            } else {
                IoEvents::empty()
            }
        })
    }
}

impl FileLike for DrmSyncFile {
    fn access_mode(&self) -> AccessMode {
        AccessMode::O_RDWR
    }

    fn path(&self) -> &Path {
        &self.pseudo_path
    }

    fn dump_proc_fdinfo(self: Arc<Self>, fd_flags: FdFlags) -> Box<dyn Display> {
        struct FdInfo {
            fd_flags: FdFlags,
        }

        impl Display for FdInfo {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                let mut flags = AccessMode::O_RDWR as u32;
                if self.fd_flags.contains(FdFlags::CLOEXEC) {
                    flags |= CreationFlags::O_CLOEXEC.bits();
                }

                writeln!(f, "pos:\t{}", 0)?;
                writeln!(f, "flags:\t0{:o}", flags)?;
                writeln!(f, "mnt_id:\t{}", AnonInodeFs::mount_node().id())?;
                writeln!(f, "ino:\t{}", AnonInodeFs::shared_inode().ino())
            }
        }

        Box::new(FdInfo { fd_flags })
    }
}

impl DrmFile {
    fn next_syncobj_handle(&self) -> u32 {
        self.next_syncobj_handle.fetch_add(1, Ordering::Relaxed)
    }

    fn insert_syncobj(&self, syncobj: Arc<DrmSyncObj>) -> Result<u32> {
        let handle = self.next_syncobj_handle();
        let mut syncobj_table = self.syncobj_table.lock();
        syncobj_table.insert(handle, syncobj);

        return Ok(handle);
    }

    pub(super) fn lookup_syncobj(&self, handle: u32) -> Result<Arc<DrmSyncObj>> {
        self.syncobj_table
            .lock()
            .get(&handle)
            .cloned()
            .ok_or(Errno::ENOENT.into())
    }

    pub(super) fn export_fence(&self, fence: Arc<DrmFence>) -> core::result::Result<i32, DrmError> {
        let current_task = Task::current().ok_or(DrmError::Invalid)?;
        let thread_local = current_task.as_thread_local().ok_or(DrmError::Invalid)?;
        let mut file_table_ref = thread_local.borrow_file_table_mut();
        let mut file_table = file_table_ref.unwrap().write();
        let fd = file_table.insert(Arc::new(DrmSyncFile::new(fence)), FdFlags::CLOEXEC);

        Ok(RawFileDesc::from(fd))
    }

    pub(super) fn import_fence(
        &self,
        fd: RawFileDesc,
    ) -> core::result::Result<Arc<DrmFence>, DrmError> {
        let fd = FileDesc::try_from(fd).map_err(|_| DrmError::Invalid)?;
        let current_task = Task::current().ok_or(DrmError::Invalid)?;
        let thread_local = current_task.as_thread_local().ok_or(DrmError::Invalid)?;
        let file_table_ref = thread_local.borrow_file_table();
        let file_table = file_table_ref.unwrap().read();
        let file = file_table.get_file(fd).map_err(|_| DrmError::Invalid)?;
        let Some(sync_file) = file.downcast_ref::<DrmSyncFile>() else {
            return Err(DrmError::Invalid);
        };

        Ok(sync_file.fence())
    }

    fn lookup_syncobjs(
        &self,
        vm: &impl VmIo,
        user_handles: u64,
        count_handles: u32,
    ) -> Result<Vec<Arc<DrmSyncObj>>> {
        if count_handles == 0 {
            return_errno!(Errno::EINVAL);
        }

        let handles = super::copy_array_from_user::<u32>(vm, user_handles, count_handles)?;

        handles
            .into_iter()
            .map(|handle| self.lookup_syncobj(handle))
            .collect()
    }

    fn remove_syncobj(&self, handle: u32) -> Result<()> {
        self.syncobj_table
            .lock()
            .remove(&handle)
            .map(|_| ())
            .ok_or(Errno::ENOENT.into())
    }

    pub(super) fn ioctl_sync_obj_create(&self, cmd: DrmIoctlSyncObjCreate) -> Result<i32> {
        let mut args = cmd.read()?;

        let flags = DrmSyncObjCreateFlags::from_bits(args.flags).ok_or(Errno::EINVAL)?;
        let syncobj = Arc::new(DrmSyncObj::new(
            flags.contains(DrmSyncObjCreateFlags::SIGNALED),
        ));

        args.handle = self.insert_syncobj(syncobj)?;

        cmd.write(&args)?;

        Ok(0)
    }

    pub(super) fn ioctl_sync_obj_destroy(&self, cmd: DrmIoctlSyncObjDestroy) -> Result<i32> {
        let args = cmd.read()?;
        if args.pad != 0 {
            return_errno!(Errno::EINVAL);
        }

        self.remove_syncobj(args.handle)?;

        Ok(0)
    }

    pub(super) fn ioctl_sync_obj_reset(&self, cmd: DrmIoctlSyncObjReset) -> Result<i32> {
        let args = cmd.read()?;
        if args.pad != 0 {
            return_errno!(Errno::EINVAL);
        }

        let syncobjs = cmd.with_data_ptr(|args_ptr| {
            self.lookup_syncobjs(args_ptr.vm(), args.handles, args.count_handles)
        })?;

        for syncobj in syncobjs {
            syncobj.reset();
        }

        Ok(0)
    }

    pub(super) fn ioctl_sync_obj_signal(&self, cmd: DrmIoctlSyncObjSignal) -> Result<i32> {
        let args = cmd.read()?;
        if args.pad != 0 {
            return_errno!(Errno::EINVAL);
        }

        let syncobjs = cmd.with_data_ptr(|args_ptr| {
            self.lookup_syncobjs(args_ptr.vm(), args.handles, args.count_handles)
        })?;

        for syncobj in syncobjs {
            syncobj.signal();
        }

        Ok(0)
    }

    pub(super) fn ioctl_sync_obj_wait(&self, cmd: DrmIoctlSyncObjWait) -> Result<i32> {
        let mut args = cmd.read()?;
        if args.pad != 0 {
            return_errno!(Errno::EINVAL);
        }

        let flags = DrmSyncObjWaitFlags::from_bits(args.flags).ok_or(Errno::EINVAL)?;
        if flags
            .intersects(DrmSyncObjWaitFlags::WAIT_AVAILABLE | DrmSyncObjWaitFlags::WAIT_DEADLINE)
        {
            return_errno!(Errno::EOPNOTSUPP);
        }

        let syncobjs = cmd.with_data_ptr(|args_ptr| {
            self.lookup_syncobjs(args_ptr.vm(), args.handles, args.count_handles)
        })?;
        let wait_all = flags.contains(DrmSyncObjWaitFlags::WAIT_ALL);

        let timeout = match args.timeout_nsec {
            -1 => None,
            value if value < -1 => {
                return_errno!(Errno::EINVAL);
            }
            value => {
                let deadline = Duration::from_nanos(value as u64);
                Some(ManagedTimeout::new_with_manager(
                    Timeout::When(deadline),
                    MonotonicClock::timer_manager(),
                ))
            }
        };

        let first_signaled = if wait_all {
            syncobjs
                .iter()
                .all(|syncobj| syncobj.is_signaled())
                .then_some(0)
        } else {
            syncobjs
                .iter()
                .position(|syncobj| syncobj.is_signaled())
                .map(|index| index as u32)
        };

        let first_signaled = match first_signaled {
            Some(first_signaled) => first_signaled,
            None => {
                let waiter = Waiter::new_pair().0;

                waiter.wait_until_or_timeout(
                    || {
                        for syncobj in &syncobjs {
                            syncobj.register_waiter(waiter.waker());
                        }

                        if wait_all {
                            syncobjs
                                .iter()
                                .all(|syncobj| syncobj.is_signaled())
                                .then_some(0)
                        } else {
                            syncobjs
                                .iter()
                                .position(|syncobj| syncobj.is_signaled())
                                .map(|index| index as u32)
                        }
                    },
                    timeout,
                )?
            }
        };
        args.first_signaled = first_signaled;
        cmd.write(&args)?;

        Ok(0)
    }
}
