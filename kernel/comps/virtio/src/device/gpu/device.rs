// SPDX-License-Identifier: MPL-2.0

use alloc::{
    boxed::Box,
    sync::{Arc, Weak},
    vec,
    vec::Vec,
};
use core::{
    sync::atomic::{AtomicU32, Ordering},
    time::Duration,
};

use aster_drm::{
    DrmAtomicOps, DrmConnStatus, DrmConnType, DrmConnector, DrmCrtc, DrmDevice, DrmDeviceBusInfo,
    DrmDeviceCaps, DrmDevicePrivate, DrmDisplayFormat, DrmDisplayInfo, DrmDisplayMode, DrmEdid,
    DrmEncoderType, DrmError, DrmFeatures, DrmFence, DrmFramebuffer, DrmGemObject, DrmGemOps,
    DrmIoctlCommandCtx, DrmIoctlEventCtx, DrmIoctlGemCtx, DrmKmsObjectBuilder, DrmKmsObjectStore,
    DrmKmsObjectType, DrmKmsOps, DrmPlane, DrmPlaneType, DrmSyncObj, DrmTaskSpawner,
    DrmVmaOffsetManager, KmsObjectId, SubpixelOrder, register_drm_device,
};
use aster_softirq::BottomHalfDisabled;
use hashbrown::{HashMap, HashSet};
use ostd::{
    arch::trap::TrapFrame,
    mm::PAGE_SIZE,
    sync::{Mutex, RwLock, SpinLock},
};
use zerocopy::IntoBytes;

use super::gem::{
    VirtioGpuGemObject, VirtioGpuHostBlobObject, VirtioGpuHostVisibleAllocation,
    VirtioGpuHostVisibleMemory,
};
use crate::{
    device::{
        VirtioDeviceError,
        gpu::{
            config::{VirtioGpuConfig, VirtioGpuFeatures},
            ioctl::*,
            queue::{control::*, header::*},
        },
    },
    queue::VirtQueue,
    transport::VirtioTransport,
};

const DEVICE_NAME: &str = "virtio_gpu";
const DRIVER_DESC: &'static str = "virtio GPU";
const VIRTIO_GPU_FALLBACK_DPI: u32 = 96;
const VIRTIO_GPU_FALLBACK_VREFRESH: u32 = 60;

const CONTROL_QUEUE_SIZE: u16 = 64;
const CURSOR_QUEUE_SIZE: u16 = 16;
const VIRTGPU_WAIT_TIMEOUT: Duration = Duration::from_secs(15);
const VIRTIO_GPU_SHM_ID_HOST_VISIBLE: u8 = 1;

#[derive(Debug, Clone)]
struct VirtioGpuContext {
    id: u32,
    context_init: u32,
    context_created: bool,
    num_rings: u32,
    ring_idx_mask: u64,
    debug_name: Vec<u8>,
    attached_resources: HashSet<u32>,
}

impl VirtioGpuContext {
    fn new(id: u32) -> Self {
        Self {
            id,
            context_init: 0,
            context_created: false,
            num_rings: 0,
            ring_idx_mask: 0,
            debug_name: Vec::new(),
            attached_resources: HashSet::new(),
        }
    }
}

#[derive(Debug)]
struct VirtioGpuPrivate {
    device: Weak<GpuDevice>,
    context: Mutex<VirtioGpuContext>,
}

impl VirtioGpuPrivate {
    fn context(&self) -> &Mutex<VirtioGpuContext> {
        &self.context
    }
}

impl DrmDevicePrivate for VirtioGpuPrivate {
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }

    fn release(&self) {
        let Some(device) = self.device.upgrade() else {
            return;
        };
        let context_id = self.context.lock().id;
        let request = VirtioGpuCtxDestroy::new(context_id);

        let _ = device.destroy_context(request);
    }
}

#[repr(u16)]
enum VirtioGpuQueue {
    QueueControl = 0,
    QueueCursor = 1,
}

#[derive(Debug)]
pub struct GpuDevice {
    weak_self: Weak<Self>,

    caps: DrmDeviceCaps,
    features: DrmFeatures,
    kms_objects: RwLock<DrmKmsObjectStore>,
    vma_manager: DrmVmaOffsetManager,

    bus_info: Option<DrmDeviceBusInfo>,
    virtio_gpu_features: VirtioGpuFeatures,
    config: VirtioGpuConfig,
    host_visible_memory: Option<Arc<VirtioGpuHostVisibleMemory>>,

    capsets: HashMap<u32, VirtioGpuCapsetInfo>,
    display_info: RwLock<HashMap<u32, VirtioGpuDisplayInfo>>,
    edids: RwLock<HashMap<u32, DrmEdid>>,

    control_queue_manager: ControlQueueManager,
    #[expect(dead_code)]
    cursor_queue: SpinLock<VirtQueue, BottomHalfDisabled>,
    transport: SpinLock<Box<dyn VirtioTransport>>,

    next_resource_id: AtomicU32,
    next_context_id: AtomicU32,
}

impl GpuDevice {
    pub(super) fn control_queue_manager(&self) -> &ControlQueueManager {
        &self.control_queue_manager
    }

    pub(super) fn next_resource_id(&self) -> u32 {
        self.next_resource_id.fetch_add(1, Ordering::Relaxed)
    }

    pub(super) fn next_context_id(&self) -> u32 {
        self.next_context_id.fetch_add(1, Ordering::Relaxed)
    }

    pub(crate) fn negotiate_features(device_features: u64) -> u64 {
        let device_features = VirtioGpuFeatures::from_bits_truncate(device_features);
        device_features.bits()
    }

    fn build_kms_objects(num_scanouts: u32) -> Result<DrmKmsObjectStore, DrmError> {
        let mut builder = DrmKmsObjectBuilder::default();

        let format_types = vec![DrmDisplayFormat::XRGB8888];

        for _ in 0..num_scanouts {
            let primary = builder.add_plane(DrmPlaneType::Primary, format_types.clone());
            let crtc = builder.add_crtc(0, primary, None);
            let encoder = builder.add_encoder(DrmEncoderType::VIRTUAL);
            let connector = builder.add_connector(DrmConnType::VIRTUAL);

            builder.plane_attach_crtc(primary, crtc)?;
            builder.encoder_attach_crtc(encoder, crtc)?;
            builder.connector_attach_encoder(connector, encoder)?;
        }

        builder.build()
    }

    pub(crate) fn init(mut transport: Box<dyn VirtioTransport>) -> Result<(), VirtioDeviceError> {
        let bus_info = transport.pci_device_id().map(|id| DrmDeviceBusInfo::Pci {
            vendor_id: id.vendor_id,
            device_id: id.device_id,
        });

        let num_queues = transport.num_queues();
        if num_queues < 2 {
            return Err(VirtioDeviceError::InvalidQueueArgs);
        }
        let mut control_queue = VirtQueue::new(
            VirtioGpuQueue::QueueControl as u16,
            CONTROL_QUEUE_SIZE,
            transport.as_mut(),
        )?;
        let cursor_queue = VirtQueue::new(
            VirtioGpuQueue::QueueCursor as u16,
            CURSOR_QUEUE_SIZE,
            transport.as_mut(),
        )?;

        let config_manager = VirtioGpuConfig::new_manager(transport.as_ref());
        let config = config_manager.read_config();

        let num_scanouts = config.num_scanouts();
        if num_scanouts > VIRTIO_GPU_MAX_SCANOUTS as u32 {
            return Err(VirtioDeviceError::InvalidQueueArgs);
        }
        let kms_objects = Self::build_kms_objects(num_scanouts)
            .map_err(|_| VirtioDeviceError::UnsupportedConfig)?;

        let virtio_gpu_features = VirtioGpuFeatures::from_bits_truncate(Self::negotiate_features(
            transport.read_device_features(),
        ));
        let host_visible_memory = transport
            .shared_memory_region(VIRTIO_GPU_SHM_ID_HOST_VISIBLE)
            .and_then(|region| {
                match VirtioGpuHostVisibleMemory::new(region.memory().clone(), region.length()) {
                    Ok(memory) => Some(memory),
                    Err(err) => {
                        ostd::warn!(
                            "virtio-gpu host-visible shared-memory region is unusable: {:?}",
                            err
                        );
                        None
                    }
                }
            });

        transport.finish_init();

        let capsets = Self::get_capset_info(config.num_capsets(), &mut control_queue)?;
        let display_info = Self::get_display_info(num_scanouts, &mut control_queue)?;
        let edids = if virtio_gpu_features.contains(VirtioGpuFeatures::EDID) {
            Self::get_edids(num_scanouts, &mut control_queue)?
        } else {
            HashMap::new()
        };

        let device = Arc::new_cyclic(|weak_device: &Weak<Self>| {
            let control_queue_manager = ControlQueueManager::new(control_queue, weak_device);

            Self {
                weak_self: weak_device.clone(),

                caps: DrmDeviceCaps::default(),
                // TODO: virtio-gpu support more features.
                features: DrmFeatures::GEM
                    | DrmFeatures::RENDER
                    | DrmFeatures::MODESET
                    | DrmFeatures::ATOMIC
                    | DrmFeatures::SYNCOBJ
                    | DrmFeatures::SYNCOBJ_TIMELINE,
                kms_objects: RwLock::new(kms_objects),
                vma_manager: DrmVmaOffsetManager::new(),

                bus_info,
                virtio_gpu_features,
                config,
                host_visible_memory,

                capsets,
                display_info: RwLock::new(display_info),
                edids: RwLock::new(edids),

                control_queue_manager,
                cursor_queue: SpinLock::new(cursor_queue),
                transport: SpinLock::new(transport),
                next_resource_id: AtomicU32::new(1),
                next_context_id: AtomicU32::new(1),
            }
        });

        let control_callback_signal = device.control_queue_manager.callback_signal();
        let handle_control_queue_irq = move |_: &TrapFrame| {
            control_callback_signal.schedule();
        };

        {
            let mut transport = device.transport.lock();
            fn config_space_change(_: &TrapFrame) {
                ostd::debug!("virtio-gpu config space changed");
            }

            transport.register_cfg_callback(Box::new(config_space_change))?;
            transport.register_queue_callback(0, Box::new(handle_control_queue_irq), false)?;
        }

        register_drm_device(device).map_err(|_| VirtioDeviceError::UnsupportedConfig)?;

        Ok(())
    }

    fn ensure_context_created(&self, ctx: &dyn DrmIoctlCommandCtx) -> Result<u32, DrmError> {
        let device_private = ctx
            .device_private()
            .and_then(|private| private.as_any().downcast_ref::<VirtioGpuPrivate>())
            .ok_or(DrmError::Invalid)?;
        let mut context = device_private.context.lock();
        let context_created = context.context_created;

        if !context_created {
            let nlen = context.debug_name.len().min(VIRTIO_GPU_MAX_DEBUG_NAME);
            let mut debug_name = [0; VIRTIO_GPU_MAX_DEBUG_NAME];
            debug_name[..nlen].copy_from_slice(&context.debug_name[..nlen]);

            let request =
                VirtioGpuCtxCreate::new(context.id, context.context_init, nlen as u32, debug_name);

            self.create_context(request)?;
            context.context_created = true;
        }

        Ok(context.id)
    }

    fn ioctl_map(&self, arg: usize, ctx: &dyn DrmIoctlCommandCtx) -> Result<(), DrmError> {
        let mut args = ctx.read_ioctl_arg::<DrmVirtgpuMap>(arg)?;
        if args.handle == 0 || args.pad != 0 {
            return Err(DrmError::Invalid);
        }

        args.offset = ctx.map_gem_handle(args.handle)?;
        ctx.write_user_bytes(arg, args.as_bytes())?;

        Ok(())
    }

    fn ioctl_execbuffer(&self, arg: usize, ctx: &dyn DrmIoctlCommandCtx) -> Result<(), DrmError> {
        let mut args = ctx.read_ioctl_arg::<DrmVirtgpuExecbuffer>(arg)?;
        let flags = VirtioGpuExecbufferFlags::from_bits(args.flags).ok_or(DrmError::Invalid)?;

        if !self.virtio_gpu_features.contains(VirtioGpuFeatures::VIRGL) {
            return Err(DrmError::Invalid);
        }

        let ring_idx = if flags.contains(VirtioGpuExecbufferFlags::RING_IDX) {
            Some(u8::try_from(args.ring_idx).map_err(|_| DrmError::Invalid)?)
        } else {
            None
        };

        let context_id = self.ensure_context_created(ctx)?;

        let command_size = usize::try_from(args.size).map_err(|_| DrmError::Invalid)?;
        if command_size > VIRTGPU_EXECBUFFER_MAX_COMMAND_SIZE {
            return Err(DrmError::Invalid);
        }
        let mut commands = vec![0; command_size];
        ctx.read_user_bytes(args.command as usize, &mut commands)?;

        let handle_count = usize::try_from(args.num_bo_handles).map_err(|_| DrmError::Invalid)?;
        if handle_count > VIRTGPU_EXECBUFFER_MAX_BO_HANDLES {
            return Err(DrmError::Invalid);
        }
        let bytes_size = handle_count
            .checked_mul(size_of::<u32>())
            .ok_or(DrmError::Invalid)?;
        let mut bytes = vec![0; bytes_size];
        ctx.read_user_bytes(args.bo_handles as usize, &mut bytes)?;

        let mut handles = Vec::with_capacity(handle_count);
        for chunk in bytes.chunks_exact(size_of::<u32>()) {
            let mut handle_bytes = [0; size_of::<u32>()];
            handle_bytes.copy_from_slice(chunk);
            handles.push(u32::from_ne_bytes(handle_bytes));
        }

        let device_private = ctx
            .device_private()
            .and_then(|private| private.as_any().downcast_ref::<VirtioGpuPrivate>())
            .ok_or(DrmError::Invalid)?;
        let mut gem_objects = Vec::with_capacity(handle_count);
        for handle in handles {
            let gem_object = ctx.lookup_gem_object(handle).ok_or(DrmError::NotFound)?;
            let resource_id = gem_object
                .as_any()
                .downcast_ref::<VirtioGpuGemObject>()
                .ok_or(DrmError::Invalid)?
                .resource_id();
            gem_objects.push(gem_object);

            let mut context = device_private.context().lock();
            let already_attached = context.attached_resources.contains(&resource_id);

            // Allow accidental repeat attach.
            if !already_attached {
                let request = VirtioGpuCtxResource::new(context_id, resource_id);
                self.attach_context_resource(request)?;
                context.attached_resources.insert(resource_id);
            }
        }

        let mut in_fences = Vec::<Arc<DrmFence>>::new();
        if flags.contains(VirtioGpuExecbufferFlags::FENCE_FD_IN) {
            in_fences.push(ctx.import_fence(args.fence_fd)?);
        }

        let syncobj_stride = if args.syncobj_stride == 0 {
            size_of::<DrmVirtgpuExecbufferSyncobj>()
        } else {
            usize::try_from(args.syncobj_stride).map_err(|_| DrmError::Invalid)?
        };
        if syncobj_stride < size_of::<DrmVirtgpuExecbufferSyncobj>() {
            return Err(DrmError::Invalid);
        }

        let in_syncobj_count =
            usize::try_from(args.num_in_syncobjs).map_err(|_| DrmError::Invalid)?;
        let out_syncobj_count =
            usize::try_from(args.num_out_syncobjs).map_err(|_| DrmError::Invalid)?;
        let total_syncobj_count = in_syncobj_count
            .checked_add(out_syncobj_count)
            .ok_or(DrmError::Invalid)?;
        if total_syncobj_count > VIRTGPU_EXECBUFFER_MAX_BO_HANDLES {
            return Err(DrmError::Invalid);
        }

        // Gather explicit synchronization dependencies before submitting commands.
        let mut reset_syncobjs = Vec::<Arc<DrmSyncObj>>::new();
        for index in 0..in_syncobj_count {
            let offset = index.checked_mul(syncobj_stride).ok_or(DrmError::Invalid)?;
            let addr = usize::try_from(args.in_syncobjs)
                .map_err(|_| DrmError::Invalid)?
                .checked_add(offset)
                .ok_or(DrmError::Invalid)?;
            let virtio_syncobj = ctx.read_ioctl_arg::<DrmVirtgpuExecbufferSyncobj>(addr)?;
            let syncobj_flags = VirtioGpuExecbufferSyncobjFlags::from_bits(virtio_syncobj.flags)
                .ok_or(DrmError::Invalid)?;
            let syncobj = ctx.lookup_syncobj(virtio_syncobj.handle)?;
            let fence = syncobj
                .fence_at(virtio_syncobj.point)
                .ok_or(DrmError::Invalid)?;
            if syncobj_flags.contains(VirtioGpuExecbufferSyncobjFlags::RESET) {
                reset_syncobjs.push(syncobj.clone());
            }
            in_fences.push(fence);
        }

        let submit_fence = Arc::new(DrmFence::new(false));
        let mut out_syncobjs = Vec::<(Arc<DrmSyncObj>, u64)>::with_capacity(out_syncobj_count);
        for index in 0..out_syncobj_count {
            let offset = index.checked_mul(syncobj_stride).ok_or(DrmError::Invalid)?;
            let addr = usize::try_from(args.out_syncobjs)
                .map_err(|_| DrmError::Invalid)?
                .checked_add(offset)
                .ok_or(DrmError::Invalid)?;
            let virtio_syncobj = ctx.read_ioctl_arg::<DrmVirtgpuExecbufferSyncobj>(addr)?;
            if virtio_syncobj.flags != 0 {
                return Err(DrmError::Invalid);
            }
            let syncobj = ctx.lookup_syncobj(virtio_syncobj.handle)?;
            out_syncobjs.push((syncobj, virtio_syncobj.point));
        }

        for fence in in_fences {
            fence.wait_timeout(Some(VIRTGPU_WAIT_TIMEOUT))?;
        }
        for syncobj in reset_syncobjs {
            syncobj.reset();
        }

        // All resources referenced by the command remain busy until the host has
        // completed the submission. Register the fence before making the command
        // visible to the device so concurrent `VIRTGPU_WAIT` calls cannot observe
        // a resource as idle after the command has been queued.
        for gem_object in &gem_objects {
            gem_object
                .as_any()
                .downcast_ref::<VirtioGpuGemObject>()
                .ok_or(DrmError::Invalid)?
                .track_fence(submit_fence.clone());
        }

        let request = VirtioGpuCmdSubmit::new(args.size, context_id, ring_idx);
        self.submit_3d_command(request, commands, Some(submit_fence.clone()))
            .map_err(|err| {
                // No host work was submitted. Wake waiters that may have observed the
                // pre-registered fence while the command was being enqueued.
                submit_fence.signal();
                err
            })?;

        for (syncobj, point) in out_syncobjs {
            syncobj.add_point(point, submit_fence.clone())?;
        }

        if flags.contains(VirtioGpuExecbufferFlags::FENCE_FD_OUT) {
            args.fence_fd = ctx.export_fence(submit_fence)?;
        }

        ctx.write_user_bytes(arg, args.as_bytes())?;

        Ok(())
    }

    fn ioctl_getparam(&self, arg: usize, ctx: &dyn DrmIoctlCommandCtx) -> Result<(), DrmError> {
        let args = ctx.read_ioctl_arg::<DrmVirtgpuGetparam>(arg)?;

        let value = match VirtioGpuParam::try_from(args.param).map_err(|_| DrmError::Invalid)? {
            VirtioGpuParam::Features3D => {
                self.virtio_gpu_features.contains(VirtioGpuFeatures::VIRGL) as u32
            }
            VirtioGpuParam::CapsetQueryFix => 1,
            VirtioGpuParam::ResourceBlob => {
                self.virtio_gpu_features
                    .contains(VirtioGpuFeatures::RESOURCE_BLOB) as u32
            }
            VirtioGpuParam::HostVisible => self.host_visible_memory.is_some() as u32,
            VirtioGpuParam::CrossDevice => {
                self.virtio_gpu_features
                    .contains(VirtioGpuFeatures::RESOURCE_UUID) as u32
            }
            VirtioGpuParam::ContextInit => {
                self.virtio_gpu_features
                    .contains(VirtioGpuFeatures::CONTEXT_INIT) as u32
            }
            VirtioGpuParam::SupportedCapsetIds => self
                .capsets
                .keys()
                .fold(0u32, |mask, &capset_id| mask | (1u32 << capset_id)),
            VirtioGpuParam::ExplicitDebugName => {
                self.virtio_gpu_features
                    .contains(VirtioGpuFeatures::CONTEXT_INIT) as u32
            }
        };

        ctx.write_user_bytes(args.value as usize, value.as_bytes())?;

        Ok(())
    }

    fn ioctl_resource_create(
        &self,
        arg: usize,
        ctx: &dyn DrmIoctlCommandCtx,
    ) -> Result<(), DrmError> {
        let mut args = ctx.read_ioctl_arg::<DrmVirtgpuResourceCreate>(arg)?;

        if !self.virtio_gpu_features.contains(VirtioGpuFeatures::VIRGL) {
            if args.depth > 1
                || args.nr_samples > 1
                || args.last_level > 1
                || args.array_size > 1
                || args.target != 2
            {
                return Err(DrmError::Invalid);
            }
        }

        let size = usize::try_from(args.size)
            .map_err(|_| DrmError::Invalid)?
            .max(PAGE_SIZE);
        let gem_object = if args.bo_handle != 0 {
            ctx.lookup_gem_object(args.bo_handle)
                .ok_or(DrmError::NotFound)?
        } else {
            ctx.create_shmem_gem(size, args.stride)?
        };

        let entries: Vec<VirtioGpuMemEntry> = gem_object
            .sg_entries()?
            .into_iter()
            .map(VirtioGpuMemEntry::from)
            .collect();

        let resource_id = self.next_resource_id();
        let fence = Arc::new(DrmFence::new(false));

        if self.virtio_gpu_features.contains(VirtioGpuFeatures::VIRGL) {
            let request = VirtioGpuResourceCreate3d::new(
                resource_id,
                args.target,
                args.format,
                args.bind,
                args.width,
                args.height,
                args.depth,
                args.array_size,
                args.last_level,
                args.nr_samples,
                args.flags,
            );

            self.create_3d_resource(request, fence.clone())?;
        } else {
            let request = VirtioGpuResourceCreate2d::new(
                resource_id,
                VirtioGpuFormat::B8G8R8X8Unorm,
                args.width,
                args.height,
            );

            self.create_2d_resource(request, fence.clone())?;
        }

        // Attach backing sg entries.
        let nr_entries = u32::try_from(entries.len()).map_err(|_| DrmError::Invalid)?;
        let request = VirtioGpuResourceAttachBacking::new(resource_id, nr_entries);
        self.attach_backing_sg_entries(request, &entries)?;

        args.res_handle = resource_id;

        let virtio_object = Arc::new(VirtioGpuGemObject::new(
            self.weak_self.clone(),
            true,
            gem_object,
            resource_id,
            Some(fence),
        ));
        args.bo_handle = if args.bo_handle != 0 {
            ctx.replace_gem_object(args.bo_handle, virtio_object)?;
            args.bo_handle
        } else {
            ctx.add_gem_object(virtio_object)?
        };

        ctx.write_user_bytes(arg, args.as_bytes())?;

        Ok(())
    }

    fn ioctl_resource_info(
        &self,
        arg: usize,
        ctx: &dyn DrmIoctlCommandCtx,
    ) -> Result<(), DrmError> {
        let mut args = ctx.read_ioctl_arg::<DrmVirtgpuResourceInfo>(arg)?;

        let gem_object = ctx
            .lookup_gem_object(args.bo_handle)
            .ok_or(DrmError::NotFound)?;

        let virtio_gem_object = gem_object
            .as_any()
            .downcast_ref::<VirtioGpuGemObject>()
            .ok_or(DrmError::Invalid)?;

        args.res_handle = virtio_gem_object.resource_id();

        args.size = u32::try_from(virtio_gem_object.size()).map_err(|_| DrmError::Invalid)?;
        args.blob_mem = virtio_gem_object
            .blob_mem()
            .map(|blob_mem| blob_mem as u32)
            .unwrap_or(0);
        ctx.write_user_bytes(arg, args.as_bytes())?;

        Ok(())
    }

    fn ioctl_transfer_from_host(
        &self,
        arg: usize,
        ctx: &dyn DrmIoctlCommandCtx,
    ) -> Result<(), DrmError> {
        let args = ctx.read_ioctl_arg::<DrmVirtgpu3dTransferFromHost>(arg)?;
        if !self.virtio_gpu_features.contains(VirtioGpuFeatures::VIRGL) {
            return Err(DrmError::FunctionNotImplemented);
        }

        let context_id = self.ensure_context_created(ctx)?;

        let gem_object = ctx
            .lookup_gem_object(args.bo_handle)
            .ok_or(DrmError::NotFound)?;
        let virtio_gem_object = gem_object
            .as_any()
            .downcast_ref::<VirtioGpuGemObject>()
            .ok_or(DrmError::Invalid)?;

        if virtio_gem_object.is_guest_only_blob() {
            return Err(DrmError::Invalid);
        }

        let request = VirtioGpuTransferHost3d::new(
            VirtioGpuCtrlType::TransferFromHost3d,
            context_id,
            virtio_gem_object.resource_id(),
            VirtioGpuBox::from(args.box_),
            args.offset as u64,
            args.level,
            args.stride,
            args.layer_stride,
        );

        let fence = Arc::new(DrmFence::new(false));
        virtio_gem_object.track_fence(fence.clone());

        self.transfer_host_3d(request, fence.clone())
            .map_err(|err| {
                fence.signal();
                err
            })?;

        Ok(())
    }

    fn ioctl_transfer_to_host(
        &self,
        arg: usize,
        ctx: &dyn DrmIoctlCommandCtx,
    ) -> Result<(), DrmError> {
        let args = ctx.read_ioctl_arg::<DrmVirtgpu3dTransferToHost>(arg)?;

        let gem_object = ctx
            .lookup_gem_object(args.bo_handle)
            .ok_or(DrmError::NotFound)?;
        let virtio_gem_object = gem_object
            .as_any()
            .downcast_ref::<VirtioGpuGemObject>()
            .ok_or(DrmError::Invalid)?;

        if virtio_gem_object.is_guest_only_blob() {
            return Err(DrmError::Invalid);
        }

        if self.virtio_gpu_features.contains(VirtioGpuFeatures::VIRGL) {
            let context_id = self.ensure_context_created(ctx)?;

            let request = VirtioGpuTransferHost3d::new(
                VirtioGpuCtrlType::TransferToHost3d,
                context_id,
                virtio_gem_object.resource_id(),
                VirtioGpuBox::from(args.box_),
                args.offset as u64,
                args.level,
                args.stride,
                args.layer_stride,
            );

            let fence = Arc::new(DrmFence::new(false));
            virtio_gem_object.track_fence(fence.clone());

            self.transfer_host_3d(request, fence.clone())
                .map_err(|err| {
                    fence.signal();
                    err
                })?;
        } else {
            let rect = VirtioGpuRect::new(args.box_.x, args.box_.y, args.box_.w, args.box_.h);
            let request = VirtioGpuTransferToHost2d::new(
                virtio_gem_object.resource_id(),
                rect,
                args.offset.into(),
            );

            self.transfer_host_2d(request)?;
        }

        Ok(())
    }

    fn map_host_visible_blob(
        &self,
        resource_id: u32,
        size: usize,
    ) -> Result<VirtioGpuHostVisibleAllocation, DrmError> {
        let host_visible_memory = self
            .host_visible_memory
            .as_ref()
            .ok_or(DrmError::NotSupported)?;
        let allocation = host_visible_memory.allocate(size)?;
        let offset = u64::try_from(allocation.offset()).map_err(|_| DrmError::Invalid)?;
        let request = VirtioGpuResourceMapBlob::new(resource_id, offset);
        let map_info = self.map_blob_resource(request)?;

        // TODO: Apply the cache policy reported in `map_info` to userspace
        // mappings. `IoMem::slice` currently preserves the PCI BAR mapping's
        // cache policy, so honoring CACHED/WC requires an API that can safely
        // derive an `IoMem` mapping with the device-selected policy.
        let _ = map_info;

        Ok(allocation)
    }

    fn ioctl_resource_create_blob(
        &self,
        arg: usize,
        ctx: &dyn DrmIoctlCommandCtx,
    ) -> Result<(), DrmError> {
        let mut args = ctx.read_ioctl_arg::<DrmVirtgpuResourceCreateBlob>(arg)?;

        if !self
            .virtio_gpu_features
            .contains(VirtioGpuFeatures::RESOURCE_BLOB)
        {
            return Err(DrmError::Invalid);
        }

        let blob_flags = VirtioGpuBlobFlags::from_bits(args.blob_flags).ok_or(DrmError::Invalid)?;
        if blob_flags.contains(VirtioGpuBlobFlags::USE_CROSS_DEVICE) {
            if !self
                .virtio_gpu_features
                .contains(VirtioGpuFeatures::RESOURCE_UUID)
            {
                return Err(DrmError::Invalid);
            }

            // TODO: Assign a UUID to the resource after creating it and keep the
            // exported-object state with the GEM object.
            return Err(DrmError::FunctionNotImplemented);
        }

        if args.pad != 0 {
            return Err(DrmError::Invalid);
        }

        let mem_flags =
            VirtioGpuBlobMemFlags::try_from(args.blob_mem).map_err(|_| DrmError::Invalid)?;
        if mem_flags == VirtioGpuBlobMemFlags::Host
            && blob_flags.contains(VirtioGpuBlobFlags::USE_MAPPABLE)
            && self.host_visible_memory.is_none()
        {
            return Err(DrmError::NotSupported);
        }

        let context_id = match mem_flags {
            VirtioGpuBlobMemFlags::Guest => {
                if args.blob_id != 0 || args.cmd_size != 0 {
                    return Err(DrmError::Invalid);
                }
                0
            }
            VirtioGpuBlobMemFlags::Host | VirtioGpuBlobMemFlags::HostGuest => {
                if !self.virtio_gpu_features.contains(VirtioGpuFeatures::VIRGL)
                    || !args.cmd_size.is_multiple_of(4)
                {
                    return Err(DrmError::Invalid);
                }

                let context_id = self.ensure_context_created(ctx)?;
                let command_size = usize::try_from(args.cmd_size).map_err(|_| DrmError::Invalid)?;
                if command_size > VIRTGPU_EXECBUFFER_MAX_COMMAND_SIZE {
                    return Err(DrmError::Invalid);
                }
                if command_size != 0 {
                    // The capset-specific command stream creates the context-local
                    // object identified by `blob_id`; the following CREATE_BLOB
                    // request binds that object to a virtio-gpu resource ID.
                    let mut commands = vec![0u8; command_size];
                    ctx.read_user_bytes(args.cmd as usize, &mut commands)?;

                    let request = VirtioGpuCmdSubmit::new(args.cmd_size, context_id, None);
                    self.submit_3d_command(request, commands, None)?;
                }

                context_id
            }
        };

        // TODO: Validate `size` against `VIRTGPU_PARAM_BLOB_ALIGNMENT` once the
        // device-specific blob alignment parameter is supported.
        let size = usize::try_from(args.size)
            .map_err(|_| DrmError::Invalid)?
            .checked_next_multiple_of(PAGE_SIZE)
            .ok_or(DrmError::Invalid)?;

        let guest_backing = match mem_flags {
            VirtioGpuBlobMemFlags::Guest | VirtioGpuBlobMemFlags::HostGuest => {
                Some(ctx.create_shmem_gem(size, 0)?)
            }
            VirtioGpuBlobMemFlags::Host => None,
        };
        let entries: Vec<VirtioGpuMemEntry> = match &guest_backing {
            Some(gem_object) => gem_object
                .sg_entries()?
                .into_iter()
                .map(VirtioGpuMemEntry::from)
                .collect(),
            None => Vec::new(),
        };
        let nr_entries = u32::try_from(entries.len()).map_err(|_| DrmError::Invalid)?;

        let resource_id = self.next_resource_id();
        let fence = Arc::new(DrmFence::new(false));
        let request = VirtioGpuResourceCreateBlob::new(
            context_id,
            resource_id,
            args.blob_mem,
            args.blob_flags,
            nr_entries,
            args.blob_id,
            args.size,
        );
        self.create_blob_resource(request, &entries, fence.clone())?;

        // Host blob + mappable.
        let gem_object: Arc<dyn DrmGemObject> = match guest_backing {
            Some(gem_object) => gem_object,
            None => {
                let mapping = if blob_flags.contains(VirtioGpuBlobFlags::USE_MAPPABLE) {
                    match self.map_host_visible_blob(resource_id, size) {
                        Ok(mapping) => Some(mapping),
                        Err(map_error) => {
                            let cleanup_object: Arc<dyn DrmGemObject> =
                                Arc::new(VirtioGpuHostBlobObject::new(size, None)?);
                            let cleanup_request = VirtioGpuResourceUnref::new(resource_id);
                            if let Err(cleanup_error) =
                                self.unref_resource(cleanup_request, cleanup_object)
                            {
                                // TODO: Queue failed resource cleanup on a normal
                                // task-context worker. The current control queue
                                // cannot retain cleanup work when it is full.
                                ostd::warn!(
                                    "virtio-gpu failed to clean up blob resource {} after map failure: {:?}",
                                    resource_id,
                                    cleanup_error
                                );
                            }
                            return Err(map_error);
                        }
                    }
                } else {
                    None
                };
                Arc::new(VirtioGpuHostBlobObject::new(size, mapping)?)
            }
        };

        let virtio_object = Arc::new(VirtioGpuGemObject::new_blob(
            self.weak_self.clone(),
            true,
            gem_object,
            resource_id,
            mem_flags,
            Some(fence),
        ));

        args.bo_handle = ctx.add_gem_object(virtio_object)?;
        args.res_handle = resource_id;

        ctx.write_user_bytes(arg, args.as_bytes())?;

        Ok(())
    }

    fn ioctl_wait(&self, arg: usize, ctx: &dyn DrmIoctlCommandCtx) -> Result<(), DrmError> {
        let args = ctx.read_ioctl_arg::<DrmVirtgpu3dWait>(arg)?;
        if args.handle == 0 {
            return Err(DrmError::Invalid);
        }

        let flags = VirtioGpuWaitFlags::from_bits(args.flags).ok_or(DrmError::Invalid)?;
        let gem_object = ctx
            .lookup_gem_object(args.handle)
            .ok_or(DrmError::NotFound)?;
        let virtio_gem_object = gem_object
            .as_any()
            .downcast_ref::<VirtioGpuGemObject>()
            .ok_or(DrmError::Invalid)?;

        if flags.contains(VirtioGpuWaitFlags::NOWAIT) {
            return (!virtio_gem_object.has_pending_fences())
                .then_some(())
                .ok_or(DrmError::Busy);
        }

        virtio_gem_object.wait_fences_timeout(Some(VIRTGPU_WAIT_TIMEOUT))
    }

    fn ioctl_get_caps(&self, arg: usize, ctx: &dyn DrmIoctlCommandCtx) -> Result<(), DrmError> {
        let args = ctx.read_ioctl_arg::<DrmVirtgpuGetCaps>(arg)?;
        if args.size == 0 {
            return Err(DrmError::Invalid);
        }
        if self.capsets.is_empty() {
            return Err(DrmError::FunctionNotImplemented);
        }

        let capset_info = self
            .capsets
            .get(&args.cap_set_id)
            .ok_or(DrmError::Invalid)?;

        if capset_info.max_version() < args.cap_set_ver {
            return Err(DrmError::Invalid);
        }

        let capset_size = usize::try_from(args.size.min(capset_info.max_size()))
            .map_err(|_| DrmError::Invalid)?;

        let request = VirtioGpuGetCapset::new(args.cap_set_id, args.cap_set_ver);
        let capsets = self.get_capsets(request, capset_size)?;

        let addr = usize::try_from(args.addr).map_err(|_| DrmError::Invalid)?;
        ctx.write_user_bytes(addr as usize, &capsets)?;

        Ok(())
    }

    fn ioctl_context_init(&self, arg: usize, ctx: &dyn DrmIoctlCommandCtx) -> Result<(), DrmError> {
        let args = ctx.read_ioctl_arg::<DrmVirtgpuContextInit>(arg)?;

        if !self
            .virtio_gpu_features
            .contains(VirtioGpuFeatures::CONTEXT_INIT | VirtioGpuFeatures::VIRGL)
        {
            return Err(DrmError::Invalid);
        }

        if args.num_params > 4 {
            return Err(DrmError::Invalid);
        }

        let device_private = ctx
            .device_private()
            .and_then(|private| private.as_any().downcast_ref::<VirtioGpuPrivate>())
            .ok_or(DrmError::Invalid)?;

        let mut context = device_private.context().lock();
        if context.context_created {
            return Err(DrmError::AlreadyExist);
        }

        for index in 0..args.num_params {
            let offset = (index as usize)
                .checked_mul(size_of::<DrmVirtgpuContextSetParam>())
                .ok_or(DrmError::Invalid)?;
            let address = (args.ctx_set_params as usize)
                .checked_add(offset)
                .ok_or(DrmError::Invalid)?;
            let param = ctx.read_ioctl_arg::<DrmVirtgpuContextSetParam>(address)?;
            let param_type =
                VirtioGpuGetParam::try_from(param.param).map_err(|_| DrmError::Invalid)?;

            match param_type {
                VirtioGpuGetParam::CapsetId => {
                    let capset_id = u32::try_from(param.value).map_err(|_| DrmError::Invalid)?;
                    if !self.capsets.contains_key(&capset_id)
                        || capset_id > VIRTIO_GPU_MAX_CAPSET_ID
                    {
                        return Err(DrmError::Invalid);
                    }

                    // A context selects one capset ID. The low 8 bits of
                    // `context_init` store that ID, so a non-zero value means
                    // the capset has already been initialized.
                    if (context.context_init & VIRTIO_GPU_CONTEXT_INIT_CAPSET_ID_MASK) != 0 {
                        return Err(DrmError::Invalid);
                    }

                    context.context_init |= capset_id;
                }
                VirtioGpuGetParam::NumRings => {
                    // TODO: base_fence_ctx
                    if context.num_rings != 0 {
                        return Err(DrmError::Invalid);
                    }

                    let num_rings = u32::try_from(param.value).map_err(|_| DrmError::Invalid)?;
                    if num_rings > VIRTGPU_MAX_RINGS {
                        return Err(DrmError::Invalid);
                    }

                    context.num_rings = num_rings;
                }
                VirtioGpuGetParam::PollRingsMask => {
                    if context.ring_idx_mask != 0 {
                        return Err(DrmError::Invalid);
                    }
                    context.ring_idx_mask = param.value;
                }
                VirtioGpuGetParam::DebugName => {
                    if !context.debug_name.is_empty() {
                        return Err(DrmError::Invalid);
                    }

                    let debug_name_addr =
                        usize::try_from(param.value).map_err(|_| DrmError::Invalid)?;
                    let mut debug_name = vec![0; VIRTGPU_DEBUG_NAME_MAX_LEN];
                    ctx.read_user_bytes(
                        debug_name_addr,
                        &mut debug_name[..VIRTGPU_DEBUG_NAME_MAX_LEN - 1],
                    )?;

                    let name_len = debug_name[..VIRTGPU_DEBUG_NAME_MAX_LEN - 1]
                        .iter()
                        .position(|&byte| byte == 0)
                        .unwrap_or(VIRTGPU_DEBUG_NAME_MAX_LEN - 1);

                    debug_name.truncate(name_len);

                    context.debug_name = debug_name;
                }
            }
        }

        let nlen = context.debug_name.len().min(VIRTIO_GPU_MAX_DEBUG_NAME);
        let mut debug_name = [0; VIRTIO_GPU_MAX_DEBUG_NAME];
        debug_name[..nlen].copy_from_slice(&context.debug_name[..nlen]);

        let request =
            VirtioGpuCtxCreate::new(context.id, context.context_init, nlen as u32, debug_name);

        self.create_context(request)?;

        context.context_created = true;

        Ok(())
    }
}

impl DrmAtomicOps for GpuDevice {
    fn atomic_flush(&self, crtc_id: KmsObjectId) -> Result<(), DrmError> {
        let (scanout_id, resource_id, is_3d, rect) = {
            let objects = self.kms_objects().read();

            let crtc = objects
                .get_object::<DrmCrtc>(crtc_id)
                .ok_or(DrmError::NotFound)?;
            if !crtc.snapshot().active() {
                return Ok(());
            }
            let scanout_id = objects
                .get_object_index(crtc_id, DrmKmsObjectType::Crtc)
                .ok_or(DrmError::NotFound)?;

            let primary = objects
                .get_object::<DrmPlane>(crtc.primary_plane_id())
                .ok_or(DrmError::NotFound)?;
            let Some(fb_id) = primary.snapshot().fb_id() else {
                return Ok(());
            };
            let fb = objects
                .get_object::<DrmFramebuffer>(fb_id)
                .ok_or(DrmError::NotFound)?;

            let gem_object = fb.gem_object(0).ok_or(DrmError::NotFound)?;
            let virtio_gem_object = gem_object
                .as_any()
                .downcast_ref::<VirtioGpuGemObject>()
                .ok_or(DrmError::Invalid)?;
            let resource_id = virtio_gem_object.resource_id();
            let is_3d = virtio_gem_object.is_3d();

            let src_rect = primary.snapshot().src_rect();
            let rect = VirtioGpuRect::new(
                src_rect.x() >> 16,
                src_rect.y() >> 16,
                src_rect.width() >> 16,
                src_rect.height() >> 16,
            );
            if rect.is_empty() {
                return Err(DrmError::Invalid);
            }

            (scanout_id, resource_id, is_3d, rect)
        };

        if !is_3d {
            let request = VirtioGpuTransferToHost2d::new(resource_id, rect, 0);
            self.transfer_host_2d(request)?;
        }

        // TODO: Not each flush should set scanout.
        // Set Scanout.
        let request = VirtioGpuSetScanout::new(scanout_id as u32, resource_id, rect);
        self.set_scanout(request)?;

        let request = VirtioGpuResourceFlush::new(resource_id, rect);
        // TODO: in plane state will given a fence.
        self.resource_flush(request)?;

        let objects = self.kms_objects().read();
        let crtc = objects
            .get_object::<DrmCrtc>(crtc_id)
            .ok_or(DrmError::NotFound)?;
        crtc.handle_vblank()?;

        Ok(())
    }
}

impl DrmKmsOps for GpuDevice {
    fn kms_objects(&self) -> &RwLock<DrmKmsObjectStore> {
        &self.kms_objects
    }

    fn update_connector_state(&self, conn_id: KmsObjectId) -> Result<(), DrmError> {
        let objects = self.kms_objects().read();
        let connector = objects
            .get_object::<DrmConnector>(conn_id)
            .ok_or(DrmError::NotFound)?;

        let scanout_index = objects
            .get_object_index(conn_id, DrmKmsObjectType::Connector)
            .ok_or(DrmError::NotFound)? as u32;
        let scanout = self
            .display_info
            .read()
            .get(&scanout_index)
            .copied()
            .ok_or(DrmError::NotFound)?;

        let snapshot = connector.snapshot();
        let encoder_id = if snapshot.encoder_id().is_some() {
            snapshot.encoder_id()
        } else {
            objects
                .collect_object_ids(
                    DrmKmsObjectType::Encoder,
                    Some(connector.possible_encoders()),
                )
                .first()
                .copied()
        };
        connector.set_current_encoder_id(encoder_id);

        if !scanout.is_enabled() {
            return connector.set_display_state(
                DrmConnStatus::Disconnected,
                vec![],
                DrmDisplayInfo::default(),
                None,
            );
        }

        let rect = scanout.rect();
        let width = rect.width.min(u16::MAX as u32) as u16;
        let height = rect.height.min(u16::MAX as u32) as u16;
        if width == 0 || height == 0 {
            return connector.set_display_state(
                DrmConnStatus::Disconnected,
                vec![],
                DrmDisplayInfo::default(),
                None,
            );
        }

        let edid_state = if self.virtio_gpu_features.contains(VirtioGpuFeatures::EDID) {
            self.edids
                .read()
                .get(&scanout_index)
                .map(|edid| (edid.modes().to_vec(), edid.display_info()))
        } else {
            None
        };

        let (mut display_modes, mut display_info) = edid_state.unwrap_or_default();
        if display_modes.is_empty() {
            display_modes = vec![DrmDisplayMode::new(
                width,
                height,
                VIRTIO_GPU_FALLBACK_VREFRESH,
            )];
        }
        if display_info.mm_width() == 0 || display_info.mm_height() == 0 {
            display_info = DrmDisplayInfo::new(
                drm_mode_res_mm(u32::from(width), VIRTIO_GPU_FALLBACK_DPI),
                drm_mode_res_mm(u32::from(height), VIRTIO_GPU_FALLBACK_DPI),
                SubpixelOrder::Unknown,
            );
        }

        connector.set_display_state(
            DrmConnStatus::Connected,
            display_modes,
            display_info,
            encoder_id,
        )
    }

    fn set_crtc(
        &self,
        crtc_id: KmsObjectId,
        fb_id: KmsObjectId,
        x: u32,
        y: u32,
        display_mode: Option<DrmDisplayMode>,
        connector_ids: Vec<KmsObjectId>,
    ) -> Result<(), DrmError> {
        self.atomic_set_crtc(crtc_id, fb_id, x, y, display_mode, connector_ids)
    }

    fn page_flip(
        &self,
        crtc_id: KmsObjectId,
        fb_id: KmsObjectId,
        user_data: u64,
        event_ctx: Arc<dyn DrmIoctlEventCtx>,
    ) -> Result<(), DrmError> {
        self.atomic_page_flip(crtc_id, fb_id, user_data, event_ctx)
    }

    fn dirty_fb(&self, fb_id: KmsObjectId) -> Result<(), DrmError> {
        self.atomic_dirty_fb(fb_id)
    }
}

impl DrmGemOps for GpuDevice {
    fn create_dumb(
        &self,
        width: u32,
        height: u32,
        bpp: u32,
        ctx: &dyn DrmIoctlGemCtx,
    ) -> Result<Arc<dyn DrmGemObject>, DrmError> {
        // The virtio-gpu only support 32 bit.
        if bpp != 32 {
            return Err(DrmError::Invalid);
        }
        if width == 0 || height == 0 {
            return Err(DrmError::Invalid);
        }
        let pitch = width.checked_mul(bpp / 8).ok_or(DrmError::Invalid)?;
        let size = pitch.checked_mul(height).ok_or(DrmError::Invalid)? as usize;

        let gem_object = ctx.create_shmem_gem(size, pitch)?;
        let entries: Vec<VirtioGpuMemEntry> = gem_object
            .sg_entries()?
            .into_iter()
            .map(VirtioGpuMemEntry::from)
            .collect();

        let fence = Arc::new(DrmFence::new(false));

        let resource_id = self.next_resource_id();
        let request = VirtioGpuResourceCreate2d::new(
            resource_id,
            VirtioGpuFormat::B8G8R8X8Unorm,
            width,
            height,
        );
        self.create_2d_resource(request, fence.clone())?;

        let nr_entries = u32::try_from(entries.len()).map_err(|_| DrmError::Invalid)?;
        let request = VirtioGpuResourceAttachBacking::new(resource_id, nr_entries);
        self.attach_backing_sg_entries(request, &entries)?;

        Ok(Arc::new(VirtioGpuGemObject::new(
            self.weak_self.clone(),
            false,
            gem_object,
            resource_id,
            Some(fence),
        )))
    }
}

impl DrmDevice for GpuDevice {
    fn init_task_context(&self, task_spawner: Arc<dyn DrmTaskSpawner>) {
        self.control_queue_manager.init_task_context(task_spawner);
    }

    fn name(&self) -> &str {
        DEVICE_NAME
    }

    fn desc(&self) -> &str {
        DRIVER_DESC
    }

    fn bus_info(&self) -> Option<DrmDeviceBusInfo> {
        self.bus_info
    }

    fn features(&self) -> &DrmFeatures {
        &self.features
    }

    fn caps(&self) -> &DrmDeviceCaps {
        &self.caps
    }

    fn vma_manager(&self) -> &DrmVmaOffsetManager {
        &self.vma_manager
    }

    fn create_private(&self) -> Result<Option<Box<dyn DrmDevicePrivate>>, DrmError> {
        let context_id = self.next_context_id();
        Ok(Some(Box::new(VirtioGpuPrivate {
            device: self.weak_self.clone(),
            context: Mutex::new(VirtioGpuContext::new(context_id)),
        })))
    }

    fn handle_command(
        &self,
        cmd: u32,
        arg: usize,
        ctx: &dyn DrmIoctlCommandCtx,
    ) -> Result<(), DrmError> {
        match cmd {
            DRM_IOCTL_VIRTGPU_MAP => self.ioctl_map(arg, ctx),
            DRM_IOCTL_VIRTGPU_EXECBUFFER => self.ioctl_execbuffer(arg, ctx),
            DRM_IOCTL_VIRTGPU_GETPARAM => self.ioctl_getparam(arg, ctx),
            DRM_IOCTL_VIRTGPU_RESOURCE_CREATE => self.ioctl_resource_create(arg, ctx),
            DRM_IOCTL_VIRTGPU_RESOURCE_INFO => self.ioctl_resource_info(arg, ctx),
            DRM_IOCTL_VIRTGPU_TRANSFER_FROM_HOST => self.ioctl_transfer_from_host(arg, ctx),
            DRM_IOCTL_VIRTGPU_TRANSFER_TO_HOST => self.ioctl_transfer_to_host(arg, ctx),
            DRM_IOCTL_VIRTGPU_WAIT => self.ioctl_wait(arg, ctx),
            DRM_IOCTL_VIRTGPU_GET_CAPS => self.ioctl_get_caps(arg, ctx),
            DRM_IOCTL_VIRTGPU_RESOURCE_CREATE_BLOB => self.ioctl_resource_create_blob(arg, ctx),
            DRM_IOCTL_VIRTGPU_CONTEXT_INIT => self.ioctl_context_init(arg, ctx),
            _ => Err(DrmError::IoctlNotFound),
        }
    }
}

fn drm_mode_res_mm(resolution_px: u32, dpi: u32) -> u32 {
    resolution_px
        .checked_mul(254)
        .and_then(|value| value.checked_div(dpi.checked_mul(10)?))
        .unwrap_or(0)
}

impl From<DrmVirtgpu3dBox> for VirtioGpuBox {
    fn from(box_: DrmVirtgpu3dBox) -> Self {
        Self {
            x: box_.x,
            y: box_.y,
            z: box_.z,
            w: box_.w,
            h: box_.h,
            d: box_.d,
        }
    }
}
