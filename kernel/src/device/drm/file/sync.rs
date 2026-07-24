// SPDX-License-Identifier: MPL-2.0

use core::{fmt::Display, sync::atomic::Ordering, time::Duration};

use aster_drm::{
    DrmError, DrmFence, DrmSyncObj, DrmSyncObjCreateFlags, DrmSyncObjQueryFlags,
    DrmSyncObjWaitCondition, DrmSyncObjWaitFlags,
};
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
    syscall::eventfd::EventFile,
    time::{clocks::MonotonicClock, timer::Timeout, wait::ManagedTimeout},
};

struct DrmSyncFile {
    inner: DrmSyncFileInner,
    pseudo_path: Path,
}

enum DrmSyncFileInner {
    Fence {
        fence: Arc<DrmFence>,
        pollee: Pollee,
    },
    SyncObj(Arc<DrmSyncObj>),
}

bitflags::bitflags! {
    struct SyncObjFdFlags: u32 {
        const SYNC_FILE = 1 << 0;
        const TIMELINE = 1 << 1;
    }
}

impl DrmSyncFile {
    fn new_fence(fence: Arc<DrmFence>) -> Self {
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
            inner: DrmSyncFileInner::Fence { fence, pollee },
            pseudo_path: AnonInodeFs::new_path(sync_file_anon_inode_path),
        }
    }

    fn new_syncobj(syncobj: Arc<DrmSyncObj>) -> Self {
        fn syncobj_anon_inode_path(_: &dyn crate::fs::vfs::inode::Inode) -> String {
            "anon_inode:[syncobj_file]".to_string()
        }

        Self {
            inner: DrmSyncFileInner::SyncObj(syncobj),
            pseudo_path: AnonInodeFs::new_path(syncobj_anon_inode_path),
        }
    }

    fn fence(&self) -> Option<Arc<DrmFence>> {
        let DrmSyncFileInner::Fence { fence, .. } = &self.inner else {
            return None;
        };

        Some(fence.clone())
    }

    fn syncobj(&self) -> Option<Arc<DrmSyncObj>> {
        let DrmSyncFileInner::SyncObj(syncobj) = &self.inner else {
            return None;
        };

        Some(syncobj.clone())
    }
}

impl Pollable for DrmSyncFile {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        match &self.inner {
            DrmSyncFileInner::Fence { fence, pollee } => pollee.poll_with(mask, poller, || {
                if fence.is_signaled() {
                    IoEvents::IN | IoEvents::OUT | IoEvents::RDNORM
                } else {
                    IoEvents::empty()
                }
            }),
            DrmSyncFileInner::SyncObj(_) => {
                (IoEvents::IN | IoEvents::OUT | IoEvents::RDNORM) & mask
            }
        }
    }
}

impl FileLike for DrmSyncFile {
    fn access_mode(&self) -> AccessMode {
        match &self.inner {
            DrmSyncFileInner::Fence { .. } => AccessMode::O_RDWR,
            DrmSyncFileInner::SyncObj(_) => AccessMode::O_RDONLY,
        }
    }

    fn path(&self) -> &Path {
        &self.pseudo_path
    }

    fn dump_proc_fdinfo(self: Arc<Self>, fd_flags: FdFlags) -> Box<dyn Display> {
        struct FdInfo {
            access_mode: AccessMode,
            fd_flags: FdFlags,
        }

        impl Display for FdInfo {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                let mut flags = self.access_mode as u32;
                if self.fd_flags.contains(FdFlags::CLOEXEC) {
                    flags |= CreationFlags::O_CLOEXEC.bits();
                }

                writeln!(f, "pos:\t{}", 0)?;
                writeln!(f, "flags:\t0{:o}", flags)?;
                writeln!(f, "mnt_id:\t{}", AnonInodeFs::mount_node().id())?;
                writeln!(f, "ino:\t{}", AnonInodeFs::shared_inode().ino())
            }
        }

        Box::new(FdInfo {
            access_mode: self.access_mode(),
            fd_flags,
        })
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
        let fd = file_table.insert(Arc::new(DrmSyncFile::new_fence(fence)), FdFlags::CLOEXEC);

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

        sync_file.fence().ok_or(DrmError::Invalid)
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

    fn wait_syncobjs(
        &self,
        syncobjs: &[Arc<DrmSyncObj>],
        points: &[u64],
        timeout_nsec: i64,
        flags: DrmSyncObjWaitFlags,
    ) -> Result<u32> {
        let wait_all = flags.contains(DrmSyncObjWaitFlags::WAIT_ALL);
        let wait_for_submit = flags.contains(DrmSyncObjWaitFlags::WAIT_FOR_SUBMIT);
        let condition = if flags.contains(DrmSyncObjWaitFlags::WAIT_AVAILABLE) {
            DrmSyncObjWaitCondition::Available
        } else {
            DrmSyncObjWaitCondition::Signaled
        };

        let deadline = Duration::from_nanos(timeout_nsec.max(0) as u64);
        let timeout = ManagedTimeout::new_with_manager(
            Timeout::When(deadline),
            MonotonicClock::timer_manager(),
        );
        let mut fences = syncobjs
            .iter()
            .zip(points)
            .map(|(syncobj, point)| syncobj.fence_at(*point))
            .collect::<Vec<_>>();
        if !wait_for_submit
            && condition != DrmSyncObjWaitCondition::Available
            && fences.iter().any(Option::is_none)
        {
            return_errno!(Errno::EINVAL);
        }

        let wait_result = |fences: &[Option<Arc<DrmFence>>]| {
            let is_ready = |fence: &Option<Arc<DrmFence>>| match condition {
                DrmSyncObjWaitCondition::Available => fence.is_some(),
                DrmSyncObjWaitCondition::Signaled => {
                    fence.as_ref().is_some_and(|fence| fence.is_signaled())
                }
            };

            if wait_all {
                fences.iter().all(is_ready).then_some(0)
            } else {
                fences.iter().position(is_ready).map(|index| index as u32)
            }
        };

        if let Some(first_signaled) = wait_result(&fences) {
            return Ok(first_signaled);
        }

        let waiter = Waiter::new_pair().0;
        waiter.wait_until_or_timeout(
            || {
                for (index, (syncobj, point)) in syncobjs.iter().zip(points).enumerate() {
                    if fences[index].is_some() {
                        if condition == DrmSyncObjWaitCondition::Signaled
                            && let Some(fence) = &fences[index]
                        {
                            fence.register_waiter(waiter.waker());
                        }
                    } else {
                        fences[index] =
                            syncobj.fence_at_and_register_waiter(*point, condition, waiter.waker());
                    }
                }

                wait_result(&fences)
            },
            timeout,
        )
    }

    fn copy_timeline_points(
        vm: &impl VmIo,
        user_points: u64,
        count_handles: u32,
    ) -> Result<Vec<u64>> {
        if user_points == 0 {
            return Ok(vec![0; count_handles as usize]);
        }

        super::copy_array_from_user::<u64>(vm, user_points, count_handles)
    }

    fn remove_syncobj(&self, handle: u32) -> Result<()> {
        self.syncobj_table
            .lock()
            .remove(&handle)
            .map(|_| ())
            .ok_or(Errno::EINVAL.into())
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

    pub(super) fn ioctl_sync_obj_handle_to_fd(
        &self,
        cmd: DrmIoctlSyncObjHandleToFd,
    ) -> Result<i32> {
        let mut args = cmd.read()?;

        if args.pad != 0 {
            return_errno!(Errno::EINVAL);
        }

        let flags = SyncObjFdFlags::from_bits(args.flags).ok_or(Errno::EINVAL)?;
        args.fd = if flags.contains(SyncObjFdFlags::SYNC_FILE) {
            let syncobj = self.lookup_syncobj(args.handle)?;
            let point = if flags.contains(SyncObjFdFlags::TIMELINE) {
                args.point
            } else {
                0
            };
            let fence = syncobj.fence_at(point).ok_or(Errno::EINVAL)?;
            self.export_fence(fence)?
        } else {
            if args.point != 0 {
                return_errno!(Errno::EINVAL);
            }

            let syncobj = self
                .lookup_syncobj(args.handle)
                .map_err(|_| Errno::EINVAL)?;
            let current_task = Task::current().ok_or(Errno::EINVAL)?;
            let thread_local = current_task.as_thread_local().ok_or(Errno::EINVAL)?;
            let mut file_table_ref = thread_local.borrow_file_table_mut();
            let mut file_table = file_table_ref.unwrap().write();
            let fd = file_table.insert(
                Arc::new(DrmSyncFile::new_syncobj(syncobj)),
                FdFlags::CLOEXEC,
            );

            RawFileDesc::from(fd)
        };
        cmd.write(&args)?;

        Ok(0)
    }

    pub(super) fn ioctl_sync_obj_fd_to_handle(
        &self,
        cmd: DrmIoctlSyncObjFdToHandle,
    ) -> Result<i32> {
        let mut args = cmd.read()?;
        if args.pad != 0 {
            return_errno!(Errno::EINVAL);
        }

        let flags = SyncObjFdFlags::from_bits(args.flags).ok_or(Errno::EINVAL)?;

        if flags.contains(SyncObjFdFlags::SYNC_FILE) {
            let fence = self.import_fence(args.fd)?;
            let point = if flags.contains(SyncObjFdFlags::TIMELINE) {
                args.point
            } else {
                0
            };
            self.lookup_syncobj(args.handle)?.add_point(point, fence)?;
        } else {
            if args.point != 0 {
                return_errno!(Errno::EINVAL);
            }

            let fd = FileDesc::try_from(args.fd).map_err(|_| Errno::EINVAL)?;
            let current_task = Task::current().ok_or(Errno::EINVAL)?;
            let thread_local = current_task.as_thread_local().ok_or(Errno::EINVAL)?;
            let file_table_ref = thread_local.borrow_file_table();
            let file_table = file_table_ref.unwrap().read();
            let file = file_table.get_file(fd).map_err(|_| Errno::EINVAL)?;
            let sync_file = file.downcast_ref::<DrmSyncFile>().ok_or(Errno::EINVAL)?;
            let syncobj = sync_file.syncobj().ok_or(Errno::EINVAL)?;

            args.handle = self.insert_syncobj(syncobj)?;
            cmd.write(&args)?;
        }

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
        if flags.contains(DrmSyncObjWaitFlags::WAIT_AVAILABLE) {
            return_errno!(Errno::EINVAL);
        }
        if args.count_handles == 0 {
            return Ok(0);
        }

        let syncobjs = cmd.with_data_ptr(|args_ptr| {
            self.lookup_syncobjs(args_ptr.vm(), args.handles, args.count_handles)
        })?;
        // TODO: Propagate `deadline_nsec` once `DrmFence` supports deadline hints.
        args.first_signaled = self.wait_syncobjs(
            &syncobjs,
            &vec![0; syncobjs.len()],
            args.timeout_nsec,
            flags,
        )?;
        cmd.write(&args)?;

        Ok(0)
    }

    pub(super) fn ioctl_sync_obj_timeline_wait(
        &self,
        cmd: DrmIoctlSyncObjTimelineWait,
    ) -> Result<i32> {
        let mut args = cmd.read()?;
        if args.pad != 0 {
            return_errno!(Errno::EINVAL);
        }

        let flags = DrmSyncObjWaitFlags::from_bits(args.flags).ok_or(Errno::EINVAL)?;
        if args.count_handles == 0 {
            return Ok(0);
        }

        let (syncobjs, points) = cmd.with_data_ptr(|args_ptr| {
            let syncobjs = self.lookup_syncobjs(args_ptr.vm(), args.handles, args.count_handles)?;
            let points =
                Self::copy_timeline_points(args_ptr.vm(), args.points, args.count_handles)?;
            Ok((syncobjs, points))
        })?;

        // TODO: Propagate `deadline_nsec` once `DrmFence` supports deadline hints.
        args.first_signaled = self.wait_syncobjs(&syncobjs, &points, args.timeout_nsec, flags)?;
        cmd.write(&args)?;

        Ok(0)
    }

    pub(super) fn ioctl_sync_obj_query(&self, cmd: DrmIoctlSyncObjQuery) -> Result<i32> {
        let args = cmd.read()?;
        if args.count_handles == 0 {
            return_errno!(Errno::EINVAL);
        }

        let flags = DrmSyncObjQueryFlags::from_bits(args.flags).ok_or(Errno::EINVAL)?;
        cmd.with_data_ptr(|args_ptr| {
            let syncobjs = self.lookup_syncobjs(args_ptr.vm(), args.handles, args.count_handles)?;
            let points = syncobjs
                .iter()
                .map(|syncobj| {
                    if flags.contains(DrmSyncObjQueryFlags::LAST_SUBMITTED) {
                        syncobj.last_submitted_point()
                    } else {
                        syncobj.last_signaled_point()
                    }
                })
                .collect::<Vec<_>>();
            args_ptr.vm().write_slice(args.points as usize, &points)?;
            Ok(())
        })?;

        Ok(0)
    }

    pub(super) fn ioctl_sync_obj_transfer(&self, cmd: DrmIoctlSyncObjTransfer) -> Result<i32> {
        let args = cmd.read()?;
        if args.pad != 0 {
            return_errno!(Errno::EINVAL);
        }

        let flags = DrmSyncObjWaitFlags::from_bits(args.flags).ok_or(Errno::EINVAL)?;
        if flags.bits() & !DrmSyncObjWaitFlags::WAIT_FOR_SUBMIT.bits() != 0 {
            return_errno!(Errno::EINVAL);
        }

        let destination = self.lookup_syncobj(args.dst_handle)?;
        let source = self.lookup_syncobj(args.src_handle)?;
        let source_fence = if flags.contains(DrmSyncObjWaitFlags::WAIT_FOR_SUBMIT) {
            source
                .wait_point_available_timeout(args.src_point, Some(Duration::from_secs(5)))
                .map_err(|error| match error {
                    DrmError::Busy => Error::new(Errno::ETIME),
                    _ => error.into(),
                })?
        } else {
            source.fence_at(args.src_point).ok_or(Errno::EINVAL)?
        };

        destination.add_point(args.dst_point, source_fence)?;
        Ok(0)
    }

    pub(super) fn ioctl_sync_obj_timeline_signal(
        &self,
        cmd: DrmIoctlSyncObjTimelineSignal,
    ) -> Result<i32> {
        let args = cmd.read()?;
        if args.flags != 0 || args.count_handles == 0 {
            return_errno!(Errno::EINVAL);
        }

        let (syncobjs, points) = cmd.with_data_ptr(|args_ptr| {
            let syncobjs = self.lookup_syncobjs(args_ptr.vm(), args.handles, args.count_handles)?;
            let points =
                Self::copy_timeline_points(args_ptr.vm(), args.points, args.count_handles)?;
            Ok((syncobjs, points))
        })?;
        for (syncobj, point) in syncobjs.iter().zip(points) {
            syncobj.signal_point(point)?;
        }

        Ok(0)
    }

    pub(super) fn ioctl_sync_obj_eventfd(&self, cmd: DrmIoctlSyncObjEventfd) -> Result<i32> {
        let args = cmd.read()?;
        if args.pad != 0 {
            return_errno!(Errno::EINVAL);
        }

        let flags = DrmSyncObjWaitFlags::from_bits(args.flags).ok_or(Errno::EINVAL)?;
        if flags.bits() & !DrmSyncObjWaitFlags::WAIT_AVAILABLE.bits() != 0 {
            return_errno!(Errno::EINVAL);
        }

        let syncobj = self.lookup_syncobj(args.handle)?;
        let fd = FileDesc::try_from(args.fd).map_err(|_| Errno::EINVAL)?;
        let event_file = {
            let current_task = Task::current().ok_or(Errno::EINVAL)?;
            let thread_local = current_task.as_thread_local().ok_or(Errno::EINVAL)?;
            let file_table_ref = thread_local.borrow_file_table();
            let file_table = file_table_ref.unwrap().read();
            file_table.get_file(fd).map_err(|_| Errno::EINVAL)?.clone()
        };
        if event_file.downcast_ref::<EventFile>().is_none() {
            return_errno!(Errno::EINVAL);
        }

        let wait_available = flags.contains(DrmSyncObjWaitFlags::WAIT_AVAILABLE);
        let point = args.point;
        // TODO: Replace this per-registration task with a native syncobj callback
        // registry to avoid retaining one sleeping task per registration.
        TaskOptions::new(move || {
            if wait_available {
                syncobj.wait_point_available(point);
            } else {
                syncobj.wait_point(point);
            }

            if let Some(event_file) = event_file.downcast_ref::<EventFile>() {
                // TODO: Match Linux eventfd overflow notification semantics.
                let _ = event_file.signal();
            }
        })
        .spawn()?;

        Ok(0)
    }
}
