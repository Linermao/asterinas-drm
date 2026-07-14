// SPDX-License-Identifier: MPL-2.0

use alloc::{
    boxed::Box,
    sync::{Arc, Weak},
    vec,
    vec::Vec,
};
use core::{
    fmt, hint,
    sync::atomic::{AtomicU32, AtomicU64, Ordering},
    time::Duration,
};

use aster_drm::{
    DrmAtomicOps, DrmConnStatus, DrmConnType, DrmConnector, DrmCrtc, DrmDevice, DrmDeviceBusInfo,
    DrmDeviceCaps, DrmDevicePrivate, DrmDisplayFormat, DrmDisplayInfo, DrmDisplayMode, DrmEdid,
    DrmEncoderType, DrmError, DrmFeatures, DrmFence, DrmFramebuffer, DrmGemObject, DrmGemOps,
    DrmIoctlCommandCtx, DrmIoctlEventCtx, DrmIoctlGemCtx, DrmKmsObjectBuilder, DrmKmsObjectStore,
    DrmKmsObjectType, DrmKmsOps, DrmPlane, DrmPlaneType, DrmSyncObj, DrmVmaOffsetManager,
    KmsObjectId, SubpixelOrder, register_drm_device,
};
use aster_softirq::{BottomHalfDisabled, Taskless};
use aster_util::mem_obj_slice::Slice;
use hashbrown::{HashMap, HashSet};
use ostd::{
    arch::trap::TrapFrame,
    mm::{HasSize, PAGE_SIZE, VmIo, dma::DmaStream},
    sync::{Mutex, RwLock, SpinLock, Waiter, Waker},
};
use zerocopy::IntoBytes;

use super::gem::VirtioGpuGemObject;
use crate::{
    device::{
        VirtioDeviceError,
        gpu::{
            VirtioGpuCommandError, VirtioGpuDeviceError,
            config::{VirtioGpuConfig, VirtioGpuFeatures},
            header::*,
            ioctl::*,
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
}

struct PendingControlCommand {
    request_slice: Slice<Arc<DmaStream>>,
    response_slice: Slice<Arc<DmaStream>>,
    completion: PendingControlCompletion,
    fence: Option<PendingControlFence>,
}

struct PendingControlFence {
    id: u64,
    fence: Arc<DrmFence>,
}

#[derive(Clone)]
struct GpuTaskless(Arc<Taskless>);

enum PendingControlCompletion {
    Sync(Arc<Waker>),
    Async,
}

impl GpuTaskless {
    fn new<F>(callback: F) -> Self
    where
        F: Fn() + Send + Sync + 'static,
    {
        Self(Taskless::new(callback))
    }

    fn schedule(&self) {
        self.0.schedule();
    }
}

impl fmt::Debug for GpuTaskless {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("GpuTaskless").finish_non_exhaustive()
    }
}

impl fmt::Debug for PendingControlCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PendingControlCommand")
            .field("completion", &self.completion)
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for PendingControlCompletion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sync(_) => f.write_str("Sync"),
            Self::Async => f.write_str("Async"),
        }
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

    capsets: HashMap<u32, VirtioGpuCapsetInfo>,
    display_info: RwLock<HashMap<u32, VirtioGpuDisplayInfo>>,
    edids: RwLock<HashMap<u32, DrmEdid>>,

    control_queue: SpinLock<VirtQueue, BottomHalfDisabled>,
    control_queue_taskless: GpuTaskless,
    #[expect(dead_code)]
    cursor_queue: SpinLock<VirtQueue, BottomHalfDisabled>,
    transport: SpinLock<Box<dyn VirtioTransport>>,

    pending_commands: SpinLock<HashMap<u16, PendingControlCommand>, BottomHalfDisabled>,

    next_resource_id: AtomicU32,
    next_context_id: AtomicU32,
    next_fence_id: AtomicU64,
}

impl GpuDevice {
    fn next_resource_id(&self) -> u32 {
        self.next_resource_id.fetch_add(1, Ordering::Relaxed)
    }

    fn next_context_id(&self) -> u32 {
        self.next_context_id.fetch_add(1, Ordering::Relaxed)
    }

    fn next_fence_id(&self) -> u64 {
        self.next_fence_id.fetch_add(1, Ordering::Relaxed)
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

    pub(crate) fn negotiate_features(device_features: u64) -> u64 {
        let device_features = VirtioGpuFeatures::from_bits_truncate(device_features);
        device_features.bits()
    }

    fn prepare_control_command(
        request_size: usize,
        response_size: usize,
    ) -> Result<(Slice<Arc<DmaStream>>, Slice<Arc<DmaStream>>), VirtioGpuCommandError> {
        let request_buffer = Arc::new(
            DmaStream::alloc(request_size.div_ceil(PAGE_SIZE), false)
                .map_err(VirtioGpuCommandError::ResourceAlloc)?,
        );
        let response_buffer = Arc::new(
            DmaStream::alloc(response_size.div_ceil(PAGE_SIZE), false)
                .map_err(VirtioGpuCommandError::ResourceAlloc)?,
        );

        let request_slice = Slice::new(request_buffer.clone(), 0..request_size);
        let response_slice = Slice::new(response_buffer.clone(), 0..response_size);

        Ok((request_slice, response_slice))
    }

    fn submit_control_command_polling(
        control_queue: &mut VirtQueue,
        request_slice: &Slice<Arc<DmaStream>>,
        response_slice: &Slice<Arc<DmaStream>>,
    ) -> Result<(), VirtioGpuCommandError> {
        request_slice
            .sync_to_device()
            .map_err(VirtioGpuCommandError::ResourceAlloc)?;
        response_slice
            .sync_to_device()
            .map_err(VirtioGpuCommandError::ResourceAlloc)?;

        let token = control_queue
            .add_dma_bufs(&[request_slice], &[response_slice])
            .map_err(|_| VirtioGpuCommandError::QueueUnavailable)?;

        if control_queue.should_notify() {
            control_queue.notify();
        }

        loop {
            match control_queue.pop_used_with_min_bytes(size_of::<VirtioGpuCtrlHdr>()) {
                Ok((completed_token, _)) if completed_token == token => break,
                Ok((completed_token, _)) => {
                    ostd::warn!(
                        "virtio-gpu completed unexpected bootstrap control command: {}",
                        completed_token
                    );
                }
                Err(_) => hint::spin_loop(),
            }
        }

        response_slice
            .sync_from_device()
            .map_err(VirtioGpuCommandError::ResourceAlloc)?;

        Ok(())
    }

    fn submit_control_command_async(
        &self,
        request_slice: &Slice<Arc<DmaStream>>,
        response_slice: &Slice<Arc<DmaStream>>,
        fence: Option<Arc<DrmFence>>,
    ) -> Result<(), VirtioGpuCommandError> {
        let pending_fence = if let Some(fence) = fence {
            let fence_id = self.next_fence_id();
            let mut request_header = request_slice
                .read_val::<VirtioGpuCtrlHdr>(0)
                .map_err(VirtioGpuCommandError::ResourceAlloc)?;

            request_header.flags |= VirtioGpuFlags::FENCE.bits();
            request_header.fence_id = fence_id;
            request_slice
                .write_val(0, &request_header)
                .map_err(VirtioGpuCommandError::ResourceAlloc)?;
            Some(PendingControlFence {
                id: fence_id,
                fence,
            })
        } else {
            None
        };

        request_slice
            .sync_to_device()
            .map_err(VirtioGpuCommandError::ResourceAlloc)?;
        response_slice
            .sync_to_device()
            .map_err(VirtioGpuCommandError::ResourceAlloc)?;

        let pending_command = PendingControlCommand {
            request_slice: request_slice.slice(0..request_slice.size()),
            response_slice: response_slice.slice(0..response_slice.size()),
            completion: PendingControlCompletion::Async,
            fence: pending_fence,
        };

        let should_notify = {
            let mut control_queue = self.control_queue.lock();

            let token = match control_queue.add_dma_bufs(&[request_slice], &[response_slice]) {
                Ok(token) => token,
                Err(_) => return Err(VirtioGpuCommandError::QueueUnavailable),
            };
            self.pending_commands.lock().insert(token, pending_command);
            let should_notify = control_queue.should_notify();

            should_notify
        };

        if should_notify {
            self.control_queue.lock().notify();
        }

        Ok(())
    }

    fn submit_control_command_sync(
        &self,
        request_slice: &Slice<Arc<DmaStream>>,
        response_slice: &Slice<Arc<DmaStream>>,
    ) -> Result<(), VirtioGpuCommandError> {
        request_slice
            .sync_to_device()
            .map_err(VirtioGpuCommandError::ResourceAlloc)?;
        response_slice
            .sync_to_device()
            .map_err(VirtioGpuCommandError::ResourceAlloc)?;

        let (waiter, waker) = Waiter::new_pair();
        let pending_command = PendingControlCommand {
            request_slice: request_slice.slice(0..request_slice.size()),
            response_slice: response_slice.slice(0..response_slice.size()),
            completion: PendingControlCompletion::Sync(waker),
            fence: None,
        };

        let should_notify = {
            let mut control_queue = self.control_queue.lock();

            let token = control_queue
                .add_dma_bufs(&[request_slice], &[response_slice])
                .map_err(|_| VirtioGpuCommandError::QueueUnavailable)?;
            self.pending_commands.lock().insert(token, pending_command);
            let should_notify = control_queue.should_notify();

            should_notify
        };

        if should_notify {
            self.control_queue.lock().notify();
        }

        waiter.wait();
        response_slice
            .sync_from_device()
            .map_err(VirtioGpuCommandError::ResourceAlloc)?;
        Ok(())
    }

    fn handle_control_queue_irq(&self) {
        loop {
            let token = {
                let mut control_queue = self.control_queue.lock();
                match control_queue.pop_used_with_min_bytes(size_of::<VirtioGpuCtrlHdr>()) {
                    Ok((token, _)) => token,
                    Err(_) => return,
                }
            };

            let Some(pending_command) = self.pending_commands.lock().remove(&token) else {
                ostd::warn!("virtio-gpu completed unknown control command");
                continue;
            };

            match pending_command.completion {
                PendingControlCompletion::Sync(waker) => {
                    waker.wake_up();
                }
                PendingControlCompletion::Async => {
                    let response_header = match pending_command.response_slice.sync_from_device() {
                        Ok(()) => match pending_command
                            .response_slice
                            .read_val::<VirtioGpuCtrlHdr>(0)
                        {
                            Ok(response_header) => Some(response_header),
                            Err(err) => {
                                ostd::warn!(
                                    "virtio-gpu failed to read async control response: {:?}",
                                    err
                                );
                                None
                            }
                        },
                        Err(err) => {
                            ostd::warn!(
                                "virtio-gpu failed to sync async control response: {:?}",
                                err
                            );
                            None
                        }
                    };

                    if let Some(response_header) = response_header
                        && response_header.type_ >= VirtioGpuCtrlType::RespErrUnspec as u32
                    {
                        let request_type = pending_command
                            .request_slice
                            .read_val::<VirtioGpuCtrlHdr>(0)
                            .ok()
                            .and_then(|header| VirtioGpuCtrlType::try_from(header.type_).ok())
                            .unwrap_or(VirtioGpuCtrlType::Unknown);
                        let response_type = VirtioGpuCtrlType::try_from(response_header.type_)
                            .unwrap_or(VirtioGpuCtrlType::Unknown);
                        ostd::warn!(
                            "virtio-gpu async control command failed: request={:?}, response={:?}, flags={:#x}, fence_id={}",
                            request_type,
                            response_type,
                            response_header.flags,
                            response_header.fence_id
                        );
                    }

                    if let Some(fence) = pending_command.fence {
                        if let Some(response_header) = response_header {
                            if response_header.flags & VirtioGpuFlags::FENCE.bits() == 0 {
                                ostd::warn!(
                                    "virtio-gpu fenced command completed without a fence response: expected_fence_id={}",
                                    fence.id
                                );
                            } else if response_header.fence_id != fence.id {
                                ostd::warn!(
                                    "virtio-gpu fenced command completed with an unexpected fence id: expected_fence_id={}, response_fence_id={}",
                                    fence.id,
                                    response_header.fence_id
                                );
                            }
                        } else {
                            ostd::warn!(
                                "virtio-gpu signaling fence {} after an unreadable async response",
                                fence.id
                            );
                        }

                        fence.fence.signal();
                    }
                }
            }
        }
    }

    fn get_capset_info(
        num_capsets: u32,
        control_queue: &mut VirtQueue,
    ) -> Result<HashMap<u32, VirtioGpuCapsetInfo>, VirtioGpuCommandError> {
        let mut capsets = HashMap::new();

        for index in 0..num_capsets {
            let (request_slice, response_slice) = Self::prepare_control_command(
                size_of::<VirtioGpuGetCapsetInfo>(),
                size_of::<VirtioGpuRespCapsetInfo>(),
            )?;

            let request = VirtioGpuGetCapsetInfo::new(index);
            request_slice
                .write_val(0, &request)
                .map_err(VirtioGpuCommandError::ResourceAlloc)?;

            Self::submit_control_command_polling(control_queue, &request_slice, &response_slice)?;

            check_response_type(&response_slice, VirtioGpuCtrlType::RespOkCapsetInfo)?;

            let response: VirtioGpuRespCapsetInfo = response_slice
                .read_val(0)
                .map_err(VirtioGpuCommandError::ResourceAlloc)?;

            let capset_info = VirtioGpuCapsetInfo::new(response);
            let capset_id = capset_info.id();
            capsets.insert(capset_id, capset_info);
        }

        Ok(capsets)
    }

    fn get_display_info(
        num_scanouts: u32,
        control_queue: &mut VirtQueue,
    ) -> Result<HashMap<u32, VirtioGpuDisplayInfo>, VirtioGpuCommandError> {
        let (request_slice, response_slice) = Self::prepare_control_command(
            size_of::<VirtioGpuCtrlHdr>(),
            size_of::<VirtioGpuRespDisplayInfo>(),
        )?;

        let request = VirtioGpuCtrlHdr::new(VirtioGpuCtrlType::GetDisplayInfo);
        request_slice
            .write_val(0, &request)
            .map_err(VirtioGpuCommandError::ResourceAlloc)?;

        Self::submit_control_command_polling(control_queue, &request_slice, &response_slice)?;

        check_response_type(&response_slice, VirtioGpuCtrlType::RespOkDisplayInfo)?;

        let response: VirtioGpuRespDisplayInfo = response_slice
            .read_val(0)
            .map_err(VirtioGpuCommandError::ResourceAlloc)?;

        let mut display_info = HashMap::new();
        for index in 0..num_scanouts {
            display_info.insert(
                index,
                VirtioGpuDisplayInfo::new(response.pmodes[index as usize]),
            );
        }

        Ok(display_info)
    }

    fn get_edids(
        num_scanouts: u32,
        control_queue: &mut VirtQueue,
    ) -> Result<HashMap<u32, DrmEdid>, VirtioGpuCommandError> {
        let mut edids = HashMap::new();

        for index in 0..num_scanouts {
            let (request_slice, response_slice) = Self::prepare_control_command(
                size_of::<VirtioGpuGetEdid>(),
                size_of::<VirtioGpuRespEdid>(),
            )?;

            let request = VirtioGpuGetEdid::new(index);
            request_slice
                .write_val(0, &request)
                .map_err(VirtioGpuCommandError::ResourceAlloc)?;

            Self::submit_control_command_polling(control_queue, &request_slice, &response_slice)?;

            check_response_type(&response_slice, VirtioGpuCtrlType::RespOkEdid)?;

            let response: VirtioGpuRespEdid = response_slice
                .read_val(0)
                .map_err(VirtioGpuCommandError::ResourceAlloc)?;

            let size =
                usize::try_from(response.size).map_err(|_| VirtioGpuCommandError::InvalidValue)?;
            if size > VIRTIO_GPU_MAX_EDID_SIZE {
                return Err(VirtioGpuCommandError::InvalidValue);
            }

            if let Ok(drm_edid) = DrmEdid::new(&response.edid[..size]) {
                edids.insert(index, drm_edid);
            }
        }

        Ok(edids)
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

        transport.finish_init();
        let capsets = Self::get_capset_info(config.num_capsets(), &mut control_queue)?;
        let display_info = Self::get_display_info(num_scanouts, &mut control_queue)?;
        let edids = if virtio_gpu_features.contains(VirtioGpuFeatures::EDID) {
            Self::get_edids(num_scanouts, &mut control_queue)?
        } else {
            HashMap::new()
        };

        let device = Arc::new_cyclic(|weak_device: &Weak<Self>| {
            let control_queue_taskless = {
                let weak_device = weak_device.clone();
                GpuTaskless::new(move || {
                    if let Some(device) = weak_device.upgrade() {
                        device.handle_control_queue_irq();
                    }
                })
            };

            Self {
                weak_self: weak_device.clone(),

                caps: DrmDeviceCaps::default(),
                // TODO: virtio-gpu support more features.
                features: DrmFeatures::GEM
                    | DrmFeatures::RENDER
                    | DrmFeatures::MODESET
                    | DrmFeatures::ATOMIC
                    | DrmFeatures::SYNCOBJ,
                kms_objects: RwLock::new(kms_objects),
                vma_manager: DrmVmaOffsetManager::new(),

                bus_info,
                virtio_gpu_features,
                config,

                capsets,
                display_info: RwLock::new(display_info),
                edids: RwLock::new(edids),

                control_queue: SpinLock::new(control_queue),
                control_queue_taskless,
                cursor_queue: SpinLock::new(cursor_queue),
                transport: SpinLock::new(transport),

                pending_commands: SpinLock::new(HashMap::new()),

                next_resource_id: AtomicU32::new(1),
                next_context_id: AtomicU32::new(1),
                next_fence_id: AtomicU64::new(1),
            }
        });

        let control_queue_taskless = device.control_queue_taskless.clone();
        let handle_control_queue_irq = move |_: &TrapFrame| {
            control_queue_taskless.schedule();
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
        let context = device_private.context.lock();

        let context_id = context.id;
        let context_init = context.context_init;
        let nlen = context.debug_name.len();
        let mut debug_name = [0; VIRTIO_GPU_MAX_DEBUG_NAME];
        debug_name[..nlen].copy_from_slice(&context.debug_name);

        let context_created = context.context_created;

        // Avoid holding the per-file context lock while submitting the host
        // command. The extra drop/relock cost is acceptable because a DRM file
        // is not expected to issue multiple asynchronous context-init requests.
        drop(context);

        if !context_created {
            let (request_slice, response_slice) = Self::prepare_control_command(
                size_of::<VirtioGpuCtxCreate>(),
                size_of::<VirtioGpuCtrlHdr>(),
            )?;

            let request =
                VirtioGpuCtxCreate::new(context_id, context_init, nlen as u32, debug_name);
            request_slice
                .write_val(0, &request)
                .map_err(VirtioGpuCommandError::ResourceAlloc)?;

            self.submit_control_command_sync(&request_slice, &response_slice)?;

            check_response_type(&response_slice, VirtioGpuCtrlType::RespOkNodata)?;
        };

        // Re-take the lock only to publish the local context state. This second
        // lock acquisition is a small and acceptable cost for context init.
        let mut context = device_private.context.lock();
        context.context_created = true;

        Ok(context_id)
    }

    fn attach_backing_sg_entries(
        &self,
        entries: &[VirtioGpuMemEntry],
        resource_id: u32,
    ) -> Result<(), VirtioGpuCommandError> {
        let entries_size = entries
            .len()
            .checked_mul(size_of::<VirtioGpuMemEntry>())
            .ok_or(VirtioGpuCommandError::InvalidValue)?;
        let request_size = size_of::<VirtioGpuResourceAttachBacking>()
            .checked_add(entries_size)
            .ok_or(VirtioGpuCommandError::InvalidValue)?;
        let (request_slice, response_slice) =
            Self::prepare_control_command(request_size, size_of::<VirtioGpuCtrlHdr>())?;

        let nr_entries =
            u32::try_from(entries.len()).map_err(|_| VirtioGpuCommandError::InvalidValue)?;
        let request = VirtioGpuResourceAttachBacking::new(resource_id, nr_entries);

        request_slice
            .write_val(0, &request)
            .map_err(VirtioGpuCommandError::ResourceAlloc)?;
        request_slice
            .write_slice(size_of::<VirtioGpuResourceAttachBacking>(), &entries)
            .map_err(VirtioGpuCommandError::ResourceAlloc)?;

        self.submit_control_command_async(&request_slice, &response_slice, None)?;

        Ok(())
    }

    pub(super) fn unref_resource(&self, resource_id: u32) -> Result<(), VirtioGpuCommandError> {
        let (request_slice, response_slice) = Self::prepare_control_command(
            size_of::<VirtioGpuResourceUnref>(),
            size_of::<VirtioGpuCtrlHdr>(),
        )?;

        let request = VirtioGpuResourceUnref::new(resource_id);
        request_slice
            .write_val(0, &request)
            .map_err(VirtioGpuCommandError::ResourceAlloc)?;

        self.submit_control_command_async(&request_slice, &response_slice, None)
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

        let context_id = self.ensure_context_created(ctx)?;

        let command_size = usize::try_from(args.size).map_err(|_| DrmError::Invalid)?;
        if command_size > VIRTGPU_EXECBUFFER_MAX_COMMAND_SIZE {
            return Err(DrmError::Invalid);
        }
        let mut command = vec![0; command_size];
        ctx.read_user_bytes(args.command as usize, &mut command)?;

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

            let context = device_private.context().lock();
            let already_attached = context.attached_resources.contains(&resource_id);
            drop(context);

            // Allow accidental repeat attach.
            if !already_attached {
                let (request_slice, response_slice) = Self::prepare_control_command(
                    size_of::<VirtioGpuCtxResource>(),
                    size_of::<VirtioGpuCtrlHdr>(),
                )?;

                let request = VirtioGpuCtxResource::new(context_id, resource_id);
                request_slice
                    .write_val(0, &request)
                    .map_err(VirtioGpuCommandError::ResourceAlloc)?;

                self.submit_control_command_sync(&request_slice, &response_slice)?;
                check_response_type(&response_slice, VirtioGpuCtrlType::RespOkNodata)?;

                let mut context = device_private.context().lock();
                context.attached_resources.insert(resource_id);
            }
        }

        let ring_idx = if flags.contains(VirtioGpuExecbufferFlags::RING_IDX) {
            Some(u8::try_from(args.ring_idx).map_err(|_| DrmError::Invalid)?)
        } else {
            None
        };

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
        let mut in_syncobjs = Vec::<Arc<DrmSyncObj>>::with_capacity(in_syncobj_count);
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
            if virtio_syncobj.point != 0 {
                return Err(DrmError::NotSupported);
            }

            let syncobj = ctx.lookup_syncobj(virtio_syncobj.handle)?;
            if syncobj_flags.contains(VirtioGpuExecbufferSyncobjFlags::RESET) {
                reset_syncobjs.push(syncobj.clone());
            }
            in_syncobjs.push(syncobj);
        }

        let submit_fence = Arc::new(DrmFence::new(false));
        let mut out_syncobjs = Vec::<Arc<DrmSyncObj>>::with_capacity(out_syncobj_count);
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
            if virtio_syncobj.point != 0 {
                return Err(DrmError::NotSupported);
            }

            let syncobj = ctx.lookup_syncobj(virtio_syncobj.handle)?;
            out_syncobjs.push(syncobj);
        }

        for fence in in_fences {
            fence.wait_timeout(Some(VIRTGPU_WAIT_TIMEOUT))?;
        }
        for syncobj in in_syncobjs {
            syncobj.wait_timeout(Some(VIRTGPU_WAIT_TIMEOUT))?;
        }
        for syncobj in reset_syncobjs {
            syncobj.reset();
        }

        // Submit 3D commands.
        let request_size = size_of::<VirtioGpuCmdSubmit>()
            .checked_add(command_size)
            .ok_or(VirtioGpuCommandError::InvalidValue)?;

        let (request_slice, response_slice) =
            Self::prepare_control_command(request_size, size_of::<VirtioGpuCtrlHdr>())?;

        let request = VirtioGpuCmdSubmit::new(command_size as u32, context_id, ring_idx);
        request_slice
            .write_val(0, &request)
            .map_err(VirtioGpuCommandError::ResourceAlloc)?;
        request_slice
            .write_bytes(size_of::<VirtioGpuCmdSubmit>(), &command)
            .map_err(VirtioGpuCommandError::ResourceAlloc)?;

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

        let submit_result = self.submit_control_command_async(
            &request_slice,
            &response_slice,
            Some(submit_fence.clone()),
        );
        if submit_result.is_err() {
            // No host work was submitted. Wake waiters that may have observed the
            // pre-registered fence while the command was being enqueued.
            submit_fence.signal();
        }
        submit_result?;

        for syncobj in out_syncobjs {
            syncobj.set_fence(submit_fence.clone());
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
                self.virtio_gpu_features.contains(VirtioGpuFeatures::VIRGL) as u64
            }
            VirtioGpuParam::CapsetQueryFix => 1,
            VirtioGpuParam::ResourceBlob => {
                self.virtio_gpu_features
                    .contains(VirtioGpuFeatures::RESOURCE_BLOB) as u64
            }
            VirtioGpuParam::HostVisible => {
                // TODO
                0
            }
            VirtioGpuParam::CrossDevice => {
                self.virtio_gpu_features
                    .contains(VirtioGpuFeatures::RESOURCE_UUID) as u64
            }
            VirtioGpuParam::ContextInit => {
                self.virtio_gpu_features
                    .contains(VirtioGpuFeatures::CONTEXT_INIT) as u64
            }
            VirtioGpuParam::SupportedCapsetIds => self
                .capsets
                .keys()
                .fold(0u64, |mask, &capset_id| mask | (1u64 << capset_id)),
            VirtioGpuParam::ExplicitDebugName => {
                self.virtio_gpu_features
                    .contains(VirtioGpuFeatures::CONTEXT_INIT) as u64
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
            // Create 3D resources.
            let (request_slice, response_slice) = Self::prepare_control_command(
                size_of::<VirtioGpuResourceCreate3d>(),
                size_of::<VirtioGpuCtrlHdr>(),
            )?;

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
            request_slice
                .write_val(0, &request)
                .map_err(VirtioGpuCommandError::ResourceAlloc)?;

            self.submit_control_command_async(
                &request_slice,
                &response_slice,
                Some(fence.clone()),
            )?;
        } else {
            // Create 2D resources.
            let (request_slice, response_slice) = Self::prepare_control_command(
                size_of::<VirtioGpuResourceCreate2d>(),
                size_of::<VirtioGpuCtrlHdr>(),
            )?;

            let request = VirtioGpuResourceCreate2d::new(
                resource_id,
                VirtioGpuFormat::B8G8R8X8Unorm,
                args.width,
                args.height,
            );

            request_slice
                .write_val(0, &request)
                .map_err(VirtioGpuCommandError::ResourceAlloc)?;

            self.submit_control_command_async(
                &request_slice,
                &response_slice,
                Some(fence.clone()),
            )?;
        }

        // Attach backing sg entries.
        self.attach_backing_sg_entries(&entries, resource_id)?;

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

        args.size = u32::try_from(gem_object.size()).map_err(|_| DrmError::Invalid)?;
        args.blob_mem = 0;
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

        let (request_slice, response_slice) = Self::prepare_control_command(
            size_of::<VirtioGpuTransferHost3d>(),
            size_of::<VirtioGpuCtrlHdr>(),
        )?;
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
        request_slice
            .write_val(0, &request)
            .map_err(VirtioGpuCommandError::ResourceAlloc)?;

        let fence = Arc::new(DrmFence::new(false));
        self.submit_control_command_async(&request_slice, &response_slice, Some(fence.clone()))?;
        virtio_gem_object.track_fence(fence);

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
        let fence = Arc::new(DrmFence::new(false));

        if self.virtio_gpu_features.contains(VirtioGpuFeatures::VIRGL) {
            let context_id = self.ensure_context_created(ctx)?;
            let (request_slice, response_slice) = Self::prepare_control_command(
                size_of::<VirtioGpuTransferHost3d>(),
                size_of::<VirtioGpuCtrlHdr>(),
            )?;
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
            request_slice
                .write_val(0, &request)
                .map_err(VirtioGpuCommandError::ResourceAlloc)?;

            self.submit_control_command_async(
                &request_slice,
                &response_slice,
                Some(fence.clone()),
            )?;
        } else {
            let rect = VirtioGpuRect::new(args.box_.x, args.box_.y, args.box_.w, args.box_.h);
            let (request_slice, response_slice) = Self::prepare_control_command(
                size_of::<VirtioGpuTransferToHost2d>(),
                size_of::<VirtioGpuCtrlHdr>(),
            )?;
            let request = VirtioGpuTransferToHost2d::new(
                virtio_gem_object.resource_id(),
                rect,
                args.offset.into(),
            );
            request_slice
                .write_val(0, &request)
                .map_err(VirtioGpuCommandError::ResourceAlloc)?;

            self.submit_control_command_async(
                &request_slice,
                &response_slice,
                Some(fence.clone()),
            )?;
        }

        virtio_gem_object.track_fence(fence);

        Ok(())
    }

    fn ioctl_resource_create_blob(
        &self,
        arg: usize,
        ctx: &dyn DrmIoctlCommandCtx,
    ) -> Result<(), DrmError> {
        let _args = ctx.read_ioctl_arg::<DrmVirtgpuResourceCreateBlob>(arg)?;

        Err(DrmError::FunctionNotImplemented)
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

        let host_capset_size =
            usize::try_from(capset_info.max_size()).map_err(|_| DrmError::Invalid)?;
        let copy_size = usize::try_from(args.size.min(capset_info.max_size()))
            .map_err(|_| DrmError::Invalid)?;
        let response_size = size_of::<VirtioGpuCtrlHdr>() + host_capset_size;
        let (request_slice, response_slice) =
            Self::prepare_control_command(size_of::<VirtioGpuGetCapset>(), response_size)?;

        let request = VirtioGpuGetCapset::new(args.cap_set_id, args.cap_set_ver);
        request_slice
            .write_val(0, &request)
            .map_err(VirtioGpuCommandError::ResourceAlloc)?;

        self.submit_control_command_sync(&request_slice, &response_slice)?;
        check_response_type(&response_slice, VirtioGpuCtrlType::RespOkCapset)?;

        let mut capsets = vec![0; copy_size];
        response_slice
            .read_bytes(size_of::<VirtioGpuCtrlHdr>(), &mut capsets)
            .map_err(VirtioGpuCommandError::ResourceAlloc)?;

        let addr = usize::try_from(args.addr).map_err(|_| DrmError::Invalid)?;

        ctx.write_user_bytes(addr as usize, &capsets)?;

        Ok(())
    }

    fn ioctl_context_init(&self, arg: usize, ctx: &dyn DrmIoctlCommandCtx) -> Result<(), DrmError> {
        let args = ctx.read_ioctl_arg::<DrmVirtgpuContextInit>(arg)?;
        if !self
            .virtio_gpu_features
            .contains(VirtioGpuFeatures::CONTEXT_INIT)
            || !self.virtio_gpu_features.contains(VirtioGpuFeatures::VIRGL)
            || args.num_params > 4
        {
            return Err(DrmError::Invalid);
        }

        let device_private = ctx
            .device_private()
            .and_then(|private| private.as_any().downcast_ref::<VirtioGpuPrivate>())
            .ok_or(DrmError::Invalid)?;

        {
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
                        let capset_id =
                            u32::try_from(param.value).map_err(|_| DrmError::Invalid)?;
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

                        let num_rings =
                            u32::try_from(param.value).map_err(|_| DrmError::Invalid)?;
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
        }

        // In this function will re-take the lock, This second
        // lock acquisition is a small and acceptable cost for context init.
        self.ensure_context_created(ctx)?;

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
            // Transfer to host 2d.
            let (request_slice, response_slice) = Self::prepare_control_command(
                size_of::<VirtioGpuTransferToHost2d>(),
                size_of::<VirtioGpuCtrlHdr>(),
            )?;
            let request = VirtioGpuTransferToHost2d::new(resource_id, rect, 0);
            request_slice
                .write_val(0, &request)
                .map_err(VirtioGpuCommandError::ResourceAlloc)?;

            self.submit_control_command_async(&request_slice, &response_slice, None)?;
        }

        // TODO: Not each flush should set scanout.
        // Set Scanout.
        let (request_slice, response_slice) = Self::prepare_control_command(
            size_of::<VirtioGpuSetScanout>(),
            size_of::<VirtioGpuCtrlHdr>(),
        )?;
        let request = VirtioGpuSetScanout::new(scanout_id as u32, resource_id, rect);
        request_slice
            .write_val(0, &request)
            .map_err(VirtioGpuCommandError::ResourceAlloc)?;

        self.submit_control_command_async(&request_slice, &response_slice, None)?;

        // Flush resources.
        let (request_slice, response_slice) = Self::prepare_control_command(
            size_of::<VirtioGpuResourceFlush>(),
            size_of::<VirtioGpuCtrlHdr>(),
        )?;
        let request = VirtioGpuResourceFlush::new(resource_id, rect);
        request_slice
            .write_val(0, &request)
            .map_err(VirtioGpuCommandError::ResourceAlloc)?;

        // TODO: in plane state will given a fence.
        self.submit_control_command_async(&request_slice, &response_slice, None)?;

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

        // Create 2D resources.
        let (request_slice, response_slice) = Self::prepare_control_command(
            size_of::<VirtioGpuResourceCreate2d>(),
            size_of::<VirtioGpuCtrlHdr>(),
        )?;

        let resource_id = self.next_resource_id();
        let request = VirtioGpuResourceCreate2d::new(
            resource_id,
            VirtioGpuFormat::B8G8R8X8Unorm,
            width,
            height,
        );

        request_slice
            .write_val(0, &request)
            .map_err(VirtioGpuCommandError::ResourceAlloc)?;

        self.submit_control_command_sync(&request_slice, &response_slice)?;
        check_response_type(&response_slice, VirtioGpuCtrlType::RespOkNodata)?;

        // Attach backing sg entries.
        let fence = Arc::new(DrmFence::new(false));
        self.attach_backing_sg_entries(&entries, resource_id)?;

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

fn check_response_type(
    response_slice: &Slice<Arc<DmaStream>>,
    expected_type: VirtioGpuCtrlType,
) -> Result<VirtioGpuCtrlHdr, VirtioGpuCommandError> {
    let response_head = response_slice
        .read_val::<VirtioGpuCtrlHdr>(0)
        .map_err(VirtioGpuCommandError::ResourceAlloc)?;

    let actual_type =
        VirtioGpuCtrlType::try_from(response_head.type_).unwrap_or(VirtioGpuCtrlType::Unknown);

    if actual_type != expected_type {
        if let Some(device_error) = VirtioGpuDeviceError::from_ctrl_type(actual_type) {
            return Err(VirtioGpuCommandError::Device(device_error));
        }

        return Err(VirtioGpuCommandError::InvalidResponseType {
            expected: expected_type,
            actual: actual_type,
        });
    }

    Ok(response_head)
}
