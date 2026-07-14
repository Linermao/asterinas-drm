// SPDX-License-Identifier: MPL-2.0

use core::{fmt::Display, sync::atomic::Ordering};

use aster_drm::{
    DRM_FORMAT_MAX_PLANES, DrmDisplayFormat, DrmError, DrmFramebuffer, DrmGemMapPage, DrmGemObject,
    DrmIoctlGemCtx, DrmKmsObject, DrmKmsObjectType, DrmPlane,
};
use ostd::task::Task;

use crate::{
    device::drm::{file::DrmFile, gem::DrmGemShmemObject, ioctl::*},
    events::IoEvents,
    fs::{
        file::{
            AccessMode, CreationFlags, FileLike, Mappable, MappableObject, MappedObject,
            file_table::{FdFlags, FileDesc, RawFileDesc},
        },
        pseudofs::AnonInodeFs,
        vfs::path::Path,
    },
    prelude::*,
    process::signal::{PollHandle, Pollable},
    vm::{perms::VmPerms, vmar::MappingHandle},
};

impl DrmIoctlGemCtx for DrmFile {
    fn create_shmem_gem(
        &self,
        size: usize,
        pitch: u32,
    ) -> core::result::Result<Arc<dyn DrmGemObject>, DrmError> {
        let obj = DrmGemShmemObject::new(size, pitch)?;
        Ok(Arc::new(obj))
    }

    fn add_gem_object(
        &self,
        gem_object: Arc<dyn DrmGemObject>,
    ) -> core::result::Result<u32, DrmError> {
        gem_object.vma_node().allow(self.file_id)?;
        let handle = self.next_gem_handle();
        self.gem_table.lock().insert(handle, gem_object);
        Ok(handle)
    }

    fn replace_gem_object(
        &self,
        handle: u32,
        gem_object: Arc<dyn DrmGemObject>,
    ) -> core::result::Result<(), DrmError> {
        gem_object.vma_node().allow(self.file_id)?;

        let mut gem_table = self.gem_table.lock();
        let old_gem_object = gem_table.get(&handle).cloned().ok_or(DrmError::NotFound)?;
        gem_table.insert(handle, gem_object.clone());
        drop(gem_table);

        if !Arc::ptr_eq(old_gem_object.vma_node(), gem_object.vma_node()) {
            old_gem_object.vma_node().revoke(self.file_id);
        }

        Ok(())
    }

    fn lookup_gem_object(&self, handle: u32) -> Option<Arc<dyn DrmGemObject>> {
        self.gem_table.lock().get(&handle).cloned()
    }

    fn map_gem_handle(&self, handle: u32) -> core::result::Result<u64, DrmError> {
        let gem_object = self.lookup_gem_object(handle).ok_or(DrmError::NotFound)?;

        let dev = self.device();
        dev.vma_manager().add(&gem_object)?;

        Ok(gem_object.vma_node().offset_addr())
    }
}

impl Mappable for DrmFile {
    fn map(&self, offset: usize, handle: MappingHandle) -> Result<Box<dyn MappedObject>> {
        if !offset.is_multiple_of(PAGE_SIZE) {
            return_errno!(Errno::EINVAL);
        }

        let map_size = handle.vm_mapping().map_size();
        let map_pages = map_size.div_ceil(PAGE_SIZE);
        let start_page = offset / PAGE_SIZE;

        let Some(gem_object) = self
            .device()
            .vma_manager()
            .lookup(start_page as u64, map_pages as u64)
        else {
            return_errno!(Errno::EFAULT);
        };

        let node = gem_object.vma_node();
        if !node.is_allowed(self.file_id) {
            return_errno!(Errno::EACCES);
        }

        let Some(object_offset) = offset.checked_sub(node.offset_addr() as usize) else {
            return_errno!(Errno::EFAULT);
        };

        Ok(Box::new(DrmGemMappedObject::new(gem_object, object_offset)))
    }
}

#[derive(Debug)]
struct DrmGemMappedObject {
    gem_object: Arc<dyn DrmGemObject>,
    object_offset: usize,
}

impl DrmGemMappedObject {
    fn new(gem_object: Arc<dyn DrmGemObject>, object_offset: usize) -> Self {
        Self {
            gem_object,
            object_offset,
        }
    }
}

impl MappedObject for DrmGemMappedObject {
    fn dup(&self) -> Box<dyn MappedObject> {
        Box::new(Self::new(self.gem_object.clone(), self.object_offset))
    }

    fn split_at(self: Box<Self>, offset: usize) -> (Box<dyn MappedObject>, Box<dyn MappedObject>) {
        let left = Box::new(Self::new(self.gem_object.clone(), self.object_offset));
        let right = Box::new(Self::new(self.gem_object, self.object_offset + offset));
        (left, right)
    }

    fn handle_page_fault(
        &self,
        offset: usize,
        _required_perms: VmPerms,
        mut handle: MappingHandle,
    ) -> Result<()> {
        let object_offset = self
            .object_offset
            .checked_add(offset)
            .ok_or_else(|| Error::new(Errno::EOVERFLOW))?;
        let page_offset = object_offset / PAGE_SIZE * PAGE_SIZE;

        match self.gem_object.map_page(page_offset)? {
            DrmGemMapPage::Frame(frame) => handle.map_frame(offset, frame),
            DrmGemMapPage::IoMem(io_mem) => handle.map_iomem(offset, io_mem),
        }

        Ok(())
    }
}

// TODO:
#[derive(Debug)]
struct DrmPrimeFile {
    gem_object: Arc<dyn DrmGemObject>,
    access_mode: AccessMode,
    pseudo_path: Path,
}

impl DrmPrimeFile {
    fn new(gem_object: Arc<dyn DrmGemObject>, access_mode: AccessMode) -> Self {
        let pseudo_path = AnonInodeFs::new_path(|_| "anon_inode:[drm-prime]".to_string());
        Self {
            gem_object,
            access_mode,
            pseudo_path,
        }
    }

    fn gem_object(&self) -> Arc<dyn DrmGemObject> {
        self.gem_object.clone()
    }
}

impl Mappable for DrmPrimeFile {
    fn map(&self, offset: usize, handle: MappingHandle) -> Result<Box<dyn MappedObject>> {
        if !offset.is_multiple_of(PAGE_SIZE) {
            return_errno!(Errno::EINVAL);
        }

        let map_size = handle.vm_mapping().map_size();
        let end_offset = offset
            .checked_add(map_size)
            .ok_or_else(|| Error::new(Errno::EOVERFLOW))?;
        if end_offset > self.gem_object().size() {
            return_errno!(Errno::EINVAL);
        }

        Ok(Box::new(DrmGemMappedObject::new(self.gem_object(), offset)))
    }
}

impl Pollable for DrmPrimeFile {
    fn poll(&self, mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        (IoEvents::IN | IoEvents::OUT | IoEvents::RDNORM) & mask
    }
}

impl FileLike for DrmPrimeFile {
    fn mappable(&self) -> Result<MappableObject<'_>> {
        Ok(MappableObject::Device(self))
    }

    fn access_mode(&self) -> AccessMode {
        self.access_mode
    }

    fn path(&self) -> &Path {
        &self.pseudo_path
    }

    fn dump_proc_fdinfo(self: Arc<Self>, fd_flags: FdFlags) -> Box<dyn Display> {
        struct FdInfo {
            inner: Arc<DrmPrimeFile>,
            fd_flags: FdFlags,
        }

        impl Display for FdInfo {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                let mut flags = self.inner.access_mode() as u32;
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
            inner: self,
            fd_flags,
        })
    }
}

impl DrmFile {
    fn next_gem_handle(&self) -> u32 {
        self.next_gem_handle.fetch_add(1, Ordering::SeqCst)
    }

    fn remove_gem_object(&self, handle: u32) -> Result<()> {
        let Some(gem_object) = self.gem_table.lock().remove(&handle) else {
            return_errno!(Errno::EINVAL);
        };

        gem_object.vma_node().revoke(self.file_id);
        Ok(())
    }

    pub(super) fn ioctl_prime_fd_to_handle(&self, cmd: DrmIoctlPrimeFdToHandle) -> Result<i32> {
        let mut args = cmd.read()?;
        let fd = FileDesc::try_from(args.fd)?;

        let gem_object = {
            let current_task = Task::current().ok_or_else(|| Error::new(Errno::ESRCH))?;
            let thread_local = current_task
                .as_thread_local()
                .ok_or_else(|| Error::new(Errno::ESRCH))?;
            let file_table_ref = thread_local.borrow_file_table();
            let file_table = file_table_ref.unwrap().read();
            let file = file_table.get_file(fd)?.clone();
            let Some(prime_file) = file.downcast_ref::<DrmPrimeFile>() else {
                return_errno!(Errno::EINVAL);
            };
            prime_file.gem_object()
        };

        args.handle = self.add_gem_object(gem_object)?;
        cmd.write(&args)?;

        Ok(0)
    }

    pub(super) fn ioctl_prime_handle_to_fd(&self, cmd: DrmIoctlPrimeHandleToFd) -> Result<i32> {
        const DRM_PRIME_RDWR: u32 = AccessMode::O_RDWR as u32;
        const DRM_PRIME_CLOEXEC: u32 = CreationFlags::O_CLOEXEC.bits();
        const DRM_PRIME_HANDLE_TO_FD_ALLOWED_FLAGS: u32 = DRM_PRIME_RDWR | DRM_PRIME_CLOEXEC;

        let mut args = cmd.read()?;
        if args.flags & !DRM_PRIME_HANDLE_TO_FD_ALLOWED_FLAGS != 0 {
            return_errno!(Errno::EINVAL);
        }

        let access_mode = if args.flags & DRM_PRIME_RDWR != 0 {
            AccessMode::O_RDWR
        } else {
            AccessMode::O_RDONLY
        };

        let fd_flags = if args.flags & DRM_PRIME_CLOEXEC != 0 {
            FdFlags::CLOEXEC
        } else {
            FdFlags::empty()
        };

        let gem_object = self.lookup_gem_object(args.handle).ok_or(Errno::ENOENT)?;
        let prime_file = DrmPrimeFile::new(gem_object, access_mode);

        let current_task = Task::current().ok_or_else(|| Error::new(Errno::ESRCH))?;
        let thread_local = current_task
            .as_thread_local()
            .ok_or_else(|| Error::new(Errno::ESRCH))?;
        let mut file_table_ref = thread_local.borrow_file_table_mut();
        let mut file_table = file_table_ref.unwrap().write();
        let fd = file_table.insert(Arc::new(prime_file), fd_flags);

        args.fd = RawFileDesc::from(fd);
        cmd.write(&args)?;

        Ok(0)
    }

    pub(super) fn ioctl_gem_close(&self, cmd: DrmIoctlGemClose) -> Result<i32> {
        let args = cmd.read()?;
        self.remove_gem_object(args.handle)?;
        Ok(0)
    }

    fn create_framebuffer(&self, fb_cmd2: &DrmModeFbCmd2) -> Result<u32> {
        const DRM_MODE_FB_INTERLACED: u32 = 1 << 0;
        const DRM_MODE_FB_MODIFIERS: u32 = 1 << 1;
        const ALLOWED_FB_FLAGS: u32 = DRM_MODE_FB_INTERLACED | DRM_MODE_FB_MODIFIERS;

        if fb_cmd2.width == 0 || fb_cmd2.height == 0 {
            return_errno!(Errno::EINVAL);
        }

        if fb_cmd2.width < self.device().caps().min_fb_width_px()
            || fb_cmd2.width > self.device().caps().max_fb_width_px()
            || fb_cmd2.height < self.device().caps().min_fb_height_px()
            || fb_cmd2.height > self.device().caps().max_fb_height_px()
        {
            return_errno!(Errno::EINVAL);
        }

        if fb_cmd2.flags & !ALLOWED_FB_FLAGS != 0 {
            return_errno!(Errno::EINVAL);
        }

        if (fb_cmd2.flags & DRM_MODE_FB_MODIFIERS != 0) && !self.device().caps().has_fb_modifiers()
        {
            return_errno!(Errno::EOPNOTSUPP);
        }

        if (fb_cmd2.flags & DRM_MODE_FB_MODIFIERS == 0) && fb_cmd2.modifier[0] != 0 {
            return_errno!(Errno::EINVAL);
        }

        // TODO: Support multi-plane framebuffer formats such as YUV/NV12.
        //
        // Linux validates each format plane through `drm_format_info::num_planes`
        // and looks up one GEM object per used plane.
        // This implementation currently accepts only single-plane formats.
        //
        for plane_index in 1..DRM_FORMAT_MAX_PLANES {
            if fb_cmd2.handles[plane_index] != 0
                || fb_cmd2.pitches[plane_index] != 0
                || fb_cmd2.offsets[plane_index] != 0
                || fb_cmd2.modifier[plane_index] != 0
            {
                return_errno!(Errno::EOPNOTSUPP);
            }
        }

        let pixel_format = DrmDisplayFormat::try_from(fb_cmd2.pixel_format)
            .map_err(|_| Error::new(Errno::EINVAL))?;

        let primary_gem = self
            .lookup_gem_object(fb_cmd2.handles[0])
            .ok_or(Errno::ENOENT)?;
        if fb_cmd2.pitches[0] == 0 {
            return_errno!(Errno::EINVAL);
        }

        let required_size = (fb_cmd2.pitches[0] as u64)
            .checked_mul(fb_cmd2.height as u64)
            .ok_or_else(|| Error::new(Errno::EOVERFLOW))?;
        let accessible_size = (primary_gem.size() as u64)
            .checked_sub(fb_cmd2.offsets[0] as u64)
            .ok_or_else(|| Error::new(Errno::EINVAL))?;
        if required_size > accessible_size {
            return_errno!(Errno::EINVAL);
        }

        let mut gems: [Option<Arc<dyn DrmGemObject>>; DRM_FORMAT_MAX_PLANES] =
            [None, None, None, None];
        gems[0] = Some(primary_gem);

        let framebuffer = DrmFramebuffer::new(
            fb_cmd2.width,
            fb_cmd2.height,
            pixel_format,
            fb_cmd2.flags,
            fb_cmd2.pitches,
            fb_cmd2.offsets,
            fb_cmd2.modifier,
            gems,
        )
        .map_err(|_| Error::new(Errno::EINVAL))?;

        let framebuffer_id = self
            .device()
            .kms_objects()
            .write()
            .add_object(DrmKmsObject::Framebuffer(framebuffer))
            .map_err(|_| Error::new(Errno::EINVAL))?;
        self.framebuffer_ids.lock().push(framebuffer_id);

        Ok(framebuffer_id)
    }

    pub(super) fn ioctl_mode_add_fb(&self, cmd: DrmIoctlModeAddFB) -> Result<i32> {
        let mut args = cmd.read()?;
        let mut fb_cmd2: DrmModeFbCmd2 = args.into();
        fb_cmd2.fb_id = 0;

        let framebuffer_id = self.create_framebuffer(&fb_cmd2)?;
        args.fb_id = framebuffer_id;

        cmd.write(&args)?;
        Ok(0)
    }

    pub(super) fn ioctl_mode_rm_fb(&self, cmd: DrmIoctlModeRmFB) -> Result<i32> {
        let framebuffer_id = cmd.read()?;

        {
            let mut framebuffer_ids = self.framebuffer_ids.lock();
            let Some(position) = framebuffer_ids
                .iter()
                .position(|existing_framebuffer_id| *existing_framebuffer_id == framebuffer_id)
            else {
                return_errno!(Errno::ENOENT);
            };
            framebuffer_ids.remove(position);
        }

        let mut objects = self.device().kms_objects().write();
        let plane_ids = objects.collect_object_ids(DrmKmsObjectType::Plane, None);
        for plane_id in plane_ids {
            let Some(plane) = objects.get_object::<DrmPlane>(plane_id) else {
                continue;
            };

            if plane.snapshot().fb_id() == Some(framebuffer_id) {
                plane.set_fb_id(None);
            }
        }
        objects
            .remove_framebuffer(framebuffer_id)
            .ok_or(Errno::ENOENT)?;

        Ok(0)
    }

    pub(super) fn ioctl_mode_create_dumb(&self, cmd: DrmIoctlModeCreateDumb) -> Result<i32> {
        if !self.device().caps().has_dumb_buffer() {
            return_errno!(Errno::ENOSYS);
        }

        let mut args = cmd.read()?;

        if args.width == 0 || args.height == 0 || args.bpp == 0 {
            return_errno!(Errno::EINVAL);
        }

        let dev = self.device();
        let gem_object = dev.create_dumb(args.width, args.height, args.bpp, self)?;
        args.pitch = gem_object.pitch();
        args.size = gem_object.size() as u64;

        args.handle = self.add_gem_object(gem_object)?;

        cmd.write(&args)?;

        Ok(0)
    }

    pub(super) fn ioctl_mode_map_dumb(&self, cmd: DrmIoctlModeMapDumb) -> Result<i32> {
        if !self.device().caps().has_dumb_buffer() {
            return_errno!(Errno::ENOSYS);
        }

        let mut args = cmd.read()?;
        args.offset = self.map_gem_handle(args.handle)?;
        cmd.write(&args)?;

        Ok(0)
    }

    pub(super) fn ioctl_mode_destroy_dumb(&self, cmd: DrmIoctlModeDestroyDumb) -> Result<i32> {
        if !self.device().caps().has_dumb_buffer() {
            return_errno!(Errno::ENOSYS);
        }

        let args = cmd.read()?;
        self.remove_gem_object(args.handle)?;

        Ok(0)
    }

    pub(super) fn ioctl_mode_add_fb2(&self, cmd: DrmIoctlModeAddFB2) -> Result<i32> {
        let mut args = cmd.read()?;
        args.fb_id = 0;

        let framebuffer_id = self.create_framebuffer(&args)?;
        args.fb_id = framebuffer_id;

        cmd.write(&args)?;
        Ok(0)
    }
}
