// SPDX-License-Identifier: MPL-2.0

use alloc::{
    collections::{BTreeMap, vec_deque::VecDeque},
    fmt,
    sync::{Arc, Weak},
    vec,
    vec::Vec,
};
use core::hint;

use aster_drm::{DrmEdid, DrmError, DrmFence, DrmGemObject};
use aster_softirq::{BottomHalfDisabled, Taskless};
use aster_util::mem_obj_slice::Slice;
use hashbrown::HashMap;
use ostd::{
    mm::{HasSize, PAGE_SIZE, VmIo, dma::DmaStream},
    sync::{Mutex, SpinLock, Waiter, Waker},
};

use crate::{
    device::gpu::{
        VirtioGpuDeviceError,
        device::GpuDevice,
        queue::{VirtioGpuCommandError, header::*},
    },
    queue::VirtQueue,
};

enum PendingControlCompletion {
    Sync(Arc<Waker>),
    Async,
}

impl fmt::Debug for PendingControlCompletion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sync(_) => f.write_str("Sync"),
            Self::Async => f.write_str("Async"),
        }
    }
}

#[derive(Debug)]
struct PendingCommand {
    request_slice: Slice<Arc<DmaStream>>,
    response_slice: Slice<Arc<DmaStream>>,
    completion: PendingControlCompletion,
    fence_id: Option<u64>,
}

pub struct ControlQueueManager {
    queue: SpinLock<VirtQueue, BottomHalfDisabled>,
    next_fence_id: Mutex<u64>,
    fence_timeline: SpinLock<BTreeMap<u64, Arc<DrmFence>>, BottomHalfDisabled>,
    pending_commands: SpinLock<HashMap<u16, PendingCommand>, BottomHalfDisabled>,
    deferred_commands: SpinLock<VecDeque<PendingCommand>, BottomHalfDisabled>,
    taskless: Arc<Taskless>,
}

impl ControlQueueManager {
    pub fn new(queue: VirtQueue, weak_device: &Weak<GpuDevice>) -> Self {
        let taskless = {
            let weak_device = weak_device.clone();
            Taskless::new(move || {
                if let Some(device) = weak_device.upgrade() {
                    device.handle_control_queue_irq();
                }
            })
        };

        Self {
            queue: SpinLock::new(queue),
            next_fence_id: Mutex::new(1),
            fence_timeline: SpinLock::new(BTreeMap::new()),
            pending_commands: SpinLock::new(HashMap::new()),
            deferred_commands: SpinLock::new(VecDeque::new()),
            taskless,
        }
    }

    pub fn taskless(&self) -> Arc<Taskless> {
        self.taskless.clone()
    }
}

impl fmt::Debug for ControlQueueManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ControlQueueManager")
            .field("queue", &self.queue)
            .field("pending_fence_count", &self.fence_timeline.lock().len())
            .field("pending_command_count", &self.pending_commands.lock().len())
            .field("deferred_unref_count", &self.deferred_commands.lock().len())
            .field("taskless", &"Taskless")
            .finish()
    }
}

impl GpuDevice {
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
        expected_type: VirtioGpuCtrlType,
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

        check_response_type(&response_slice, expected_type)?;

        Ok(())
    }

    fn submit_control_command_async(
        &self,
        request_slice: &Slice<Arc<DmaStream>>,
        response_slice: &Slice<Arc<DmaStream>>,
        fence: Option<Arc<DrmFence>>,
    ) -> Result<(), VirtioGpuCommandError> {
        request_slice
            .sync_to_device()
            .map_err(VirtioGpuCommandError::ResourceAlloc)?;
        response_slice
            .sync_to_device()
            .map_err(VirtioGpuCommandError::ResourceAlloc)?;

        // Hold this across preparation and insertion so fence ID order follows
        // fenced command order, without performing DMA synchronization under the
        // control-queue spin lock.
        let queue_manager = self.control_queue_manager();
        let mut fence_guard = queue_manager.next_fence_id.lock();

        let pending_fence = if let Some(fence) = fence {
            let next_fence_id = *fence_guard;

            let mut request_header = request_slice
                .read_val::<VirtioGpuCtrlHdr>(0)
                .map_err(VirtioGpuCommandError::ResourceAlloc)?;
            request_header.flags |= VirtioGpuFlags::FENCE.bits();
            request_header.fence_id = next_fence_id;
            request_slice
                .write_val(0, &request_header)
                .map_err(VirtioGpuCommandError::ResourceAlloc)?;
            request_slice
                .sync_to_device()
                .map_err(VirtioGpuCommandError::ResourceAlloc)?;

            *fence_guard += 1;

            Some((next_fence_id, fence))
        } else {
            None
        };

        let pending_command = PendingCommand {
            request_slice: request_slice.slice(0..request_slice.size()),
            response_slice: response_slice.slice(0..response_slice.size()),
            completion: PendingControlCompletion::Async,
            fence_id: pending_fence.as_ref().map(|(fence_id, _)| *fence_id),
        };

        let mut control_queue = queue_manager.queue.lock();
        let token = control_queue
            .add_dma_bufs(
                &[&pending_command.request_slice],
                &[&pending_command.response_slice],
            )
            .map_err(|_| VirtioGpuCommandError::QueueUnavailable)?;

        queue_manager
            .pending_commands
            .lock()
            .insert(token, pending_command);
        if let Some((fence_id, fence)) = pending_fence {
            queue_manager.fence_timeline.lock().insert(fence_id, fence);
        }

        if control_queue.should_notify() {
            control_queue.notify();
        }

        Ok(())
    }

    fn submit_control_command_sync(
        &self,
        request_slice: &Slice<Arc<DmaStream>>,
        response_slice: &Slice<Arc<DmaStream>>,
        expected_type: VirtioGpuCtrlType,
    ) -> Result<(), VirtioGpuCommandError> {
        request_slice
            .sync_to_device()
            .map_err(VirtioGpuCommandError::ResourceAlloc)?;
        response_slice
            .sync_to_device()
            .map_err(VirtioGpuCommandError::ResourceAlloc)?;

        let (waiter, waker) = Waiter::new_pair();
        let pending_command = PendingCommand {
            request_slice: request_slice.slice(0..request_slice.size()),
            response_slice: response_slice.slice(0..response_slice.size()),
            completion: PendingControlCompletion::Sync(waker),
            fence_id: None,
        };

        let queue_manager = self.control_queue_manager();
        {
            let mut control_queue = queue_manager.queue.lock();

            let token = control_queue
                .add_dma_bufs(&[request_slice], &[response_slice])
                .map_err(|_| VirtioGpuCommandError::QueueUnavailable)?;
            queue_manager
                .pending_commands
                .lock()
                .insert(token, pending_command);

            if control_queue.should_notify() {
                control_queue.notify();
            }
        }

        waiter.wait();
        response_slice
            .sync_from_device()
            .map_err(VirtioGpuCommandError::ResourceAlloc)?;

        check_response_type(&response_slice, expected_type)?;

        Ok(())
    }

    fn handle_control_queue_irq(&self) {
        loop {
            let queue_manager = self.control_queue_manager();
            let token = {
                let mut control_queue = queue_manager.queue.lock();
                match control_queue.pop_used_with_min_bytes(size_of::<VirtioGpuCtrlHdr>()) {
                    Ok((token, _)) => token,
                    Err(_) => return,
                }
            };

            let Some(pending_command) = queue_manager.pending_commands.lock().remove(&token) else {
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

                    if let Some(response_header) = response_header {
                        if response_header.flags & VirtioGpuFlags::FENCE.bits() != 0 {
                            let signaled_fences = {
                                let mut fence_timeline = queue_manager.fence_timeline.lock();

                                let fence_id = response_header.fence_id;
                                if !fence_timeline.contains_key(&fence_id) {
                                    ostd::warn!(
                                        "virtio-gpu completed an unknown fence: response_fence_id={}",
                                        fence_id
                                    );
                                }

                                let later_fences = match fence_id.checked_add(1) {
                                    Some(next_fence_id) => fence_timeline.split_off(&next_fence_id),
                                    None => BTreeMap::new(),
                                };
                                core::mem::replace(&mut *fence_timeline, later_fences)
                            };

                            for (_, fence) in signaled_fences {
                                fence.signal();
                            }
                        } else if let Some(expected_fence_id) = pending_command.fence_id {
                            ostd::warn!(
                                "virtio-gpu fenced command completed without a fence response: expected_fence_id={}",
                                expected_fence_id
                            );
                        }
                    } else if let Some(expected_fence_id) = pending_command.fence_id {
                        ostd::warn!(
                            "virtio-gpu fenced command completed with an unreadable response: expected_fence_id={}",
                            expected_fence_id
                        );
                    }
                }
            }

            self.retry_deferred_unrefs();
        }
    }

    fn retry_deferred_unrefs(&self) {
        let queue_manager = self.control_queue_manager();
        let mut control_queue = queue_manager.queue.lock();
        let Some(pending_command) = queue_manager.deferred_commands.lock().pop_front() else {
            return;
        };

        let token = match control_queue.add_dma_bufs(
            &[&pending_command.request_slice],
            &[&pending_command.response_slice],
        ) {
            Ok(token) => token,
            Err(_) => {
                queue_manager
                    .deferred_commands
                    .lock()
                    .push_front(pending_command);
                return;
            }
        };
        queue_manager
            .pending_commands
            .lock()
            .insert(token, pending_command);

        if control_queue.should_notify() {
            control_queue.notify();
        }
    }

    pub fn set_scanout(&self, request: VirtioGpuSetScanout) -> Result<(), VirtioGpuCommandError> {
        let (request_slice, response_slice) = Self::prepare_control_command(
            size_of::<VirtioGpuSetScanout>(),
            size_of::<VirtioGpuCtrlHdr>(),
        )?;
        request_slice
            .write_val(0, &request)
            .map_err(VirtioGpuCommandError::ResourceAlloc)?;

        self.submit_control_command_async(&request_slice, &response_slice, None)?;

        Ok(())
    }

    pub fn resource_flush(
        &self,
        request: VirtioGpuResourceFlush,
    ) -> Result<(), VirtioGpuCommandError> {
        let (request_slice, response_slice) = Self::prepare_control_command(
            size_of::<VirtioGpuResourceFlush>(),
            size_of::<VirtioGpuCtrlHdr>(),
        )?;
        request_slice
            .write_val(0, &request)
            .map_err(VirtioGpuCommandError::ResourceAlloc)?;

        self.submit_control_command_sync(
            &request_slice,
            &response_slice,
            VirtioGpuCtrlType::RespOkNodata,
        )?;

        Ok(())
    }

    pub fn get_capset_info(
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

            Self::submit_control_command_polling(
                control_queue,
                &request_slice,
                &response_slice,
                VirtioGpuCtrlType::RespOkCapsetInfo,
            )?;

            let response: VirtioGpuRespCapsetInfo = response_slice
                .read_val(0)
                .map_err(VirtioGpuCommandError::ResourceAlloc)?;

            let capset_info = VirtioGpuCapsetInfo::new(response);
            let capset_id = capset_info.id();
            capsets.insert(capset_id, capset_info);
        }

        Ok(capsets)
    }

    pub fn get_display_info(
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

        Self::submit_control_command_polling(
            control_queue,
            &request_slice,
            &response_slice,
            VirtioGpuCtrlType::RespOkDisplayInfo,
        )?;

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

    pub fn get_edids(
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

            Self::submit_control_command_polling(
                control_queue,
                &request_slice,
                &response_slice,
                VirtioGpuCtrlType::RespOkEdid,
            )?;

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

    pub fn get_capsets(
        &self,
        request: VirtioGpuGetCapset,
        capset_size: usize,
    ) -> Result<Vec<u8>, DrmError> {
        let (request_slice, response_slice) = Self::prepare_control_command(
            size_of::<VirtioGpuGetCapset>(),
            size_of::<VirtioGpuCtrlHdr>() + capset_size,
        )?;
        request_slice
            .write_val(0, &request)
            .map_err(VirtioGpuCommandError::ResourceAlloc)?;

        self.submit_control_command_sync(
            &request_slice,
            &response_slice,
            VirtioGpuCtrlType::RespOkCapset,
        )?;

        let mut capsets = vec![0; capset_size];
        response_slice
            .read_bytes(size_of::<VirtioGpuCtrlHdr>(), &mut capsets)
            .map_err(VirtioGpuCommandError::ResourceAlloc)?;
        Ok(capsets)
    }

    pub fn create_context(&self, request: VirtioGpuCtxCreate) -> Result<(), VirtioGpuCommandError> {
        let (request_slice, response_slice) = Self::prepare_control_command(
            size_of::<VirtioGpuCtxCreate>(),
            size_of::<VirtioGpuCtrlHdr>(),
        )?;

        request_slice
            .write_val(0, &request)
            .map_err(VirtioGpuCommandError::ResourceAlloc)?;

        self.submit_control_command_async(&request_slice, &response_slice, None)?;

        Ok(())
    }

    pub fn destroy_context(&self, request: VirtioGpuCtxDestroy) -> Result<(), VirtioGpuCommandError> {
        let (request_slice, response_slice) = Self::prepare_control_command(
            size_of::<VirtioGpuCtxDestroy>(),
            size_of::<VirtioGpuCtrlHdr>(),
        )?;

        request_slice
            .write_val(0, &request)
            .map_err(VirtioGpuCommandError::ResourceAlloc)?;

        self.submit_control_command_async(&request_slice, &response_slice, None)?;
        
        Ok(())
    }

    pub fn create_2d_resource(
        &self,
        request: VirtioGpuResourceCreate2d,
        fence: Arc<DrmFence>,
    ) -> Result<(), VirtioGpuCommandError> {
        let (request_slice, response_slice) = Self::prepare_control_command(
            size_of::<VirtioGpuResourceCreate2d>(),
            size_of::<VirtioGpuCtrlHdr>(),
        )?;

        request_slice
            .write_val(0, &request)
            .map_err(VirtioGpuCommandError::ResourceAlloc)?;

        self.submit_control_command_async(&request_slice, &response_slice, Some(fence))?;

        Ok(())
    }

    pub fn create_3d_resource(
        &self,
        request: VirtioGpuResourceCreate3d,
        fence: Arc<DrmFence>,
    ) -> Result<(), VirtioGpuCommandError> {
        let (request_slice, response_slice) = Self::prepare_control_command(
            size_of::<VirtioGpuResourceCreate3d>(),
            size_of::<VirtioGpuCtrlHdr>(),
        )?;

        request_slice
            .write_val(0, &request)
            .map_err(VirtioGpuCommandError::ResourceAlloc)?;

        self.submit_control_command_async(&request_slice, &response_slice, Some(fence.clone()))?;

        Ok(())
    }

    pub fn attach_backing_sg_entries(
        &self,
        request: VirtioGpuResourceAttachBacking,
        entries: &[VirtioGpuMemEntry],
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

        request_slice
            .write_val(0, &request)
            .map_err(VirtioGpuCommandError::ResourceAlloc)?;
        request_slice
            .write_slice(size_of::<VirtioGpuResourceAttachBacking>(), &entries)
            .map_err(VirtioGpuCommandError::ResourceAlloc)?;

        self.submit_control_command_async(&request_slice, &response_slice, None)?;

        Ok(())
    }

    pub fn detach_backing_sg_entries (
        &self,
        request: VirtioGpuResourceDetachBacking,
    ) -> Result<(), VirtioGpuCommandError> {
        let (request_slice, response_slice) = Self::prepare_control_command(
            size_of::<VirtioGpuResourceDetachBacking>(),
            size_of::<VirtioGpuCtrlHdr>(),
        )?;

        request_slice
            .write_val(0, &request)
            .map_err(VirtioGpuCommandError::ResourceAlloc)?;

        self.submit_control_command_async(
            &request_slice,
            &response_slice,
            None,
        )?;

        Ok(())
    }


    pub fn attach_context_resource(
        &self,
        request: VirtioGpuCtxResource,
    ) -> Result<(), VirtioGpuCommandError> {
        let (request_slice, response_slice) = Self::prepare_control_command(
            size_of::<VirtioGpuCtxResource>(),
            size_of::<VirtioGpuCtrlHdr>(),
        )?;

        request_slice
            .write_val(0, &request)
            .map_err(VirtioGpuCommandError::ResourceAlloc)?;

        self.submit_control_command_sync(
            &request_slice,
            &response_slice,
            VirtioGpuCtrlType::RespOkNodata,
        )?;

        Ok(())
    }

    pub fn submit_3d_command(
        &self,
        request: VirtioGpuCmdSubmit,
        commands: Vec<u8>,
        fence: Arc<DrmFence>,
    ) -> Result<(), VirtioGpuCommandError> {
        let request_size = size_of::<VirtioGpuCmdSubmit>()
            .checked_add(commands.len())
            .ok_or(VirtioGpuCommandError::InvalidValue)?;
        let (request_slice, response_slice) =
            Self::prepare_control_command(request_size, size_of::<VirtioGpuCtrlHdr>())?;

        request_slice
            .write_val(0, &request)
            .map_err(VirtioGpuCommandError::ResourceAlloc)?;
        request_slice
            .write_bytes(size_of::<VirtioGpuCmdSubmit>(), &commands)
            .map_err(VirtioGpuCommandError::ResourceAlloc)?;

        self.submit_control_command_async(&request_slice, &response_slice, Some(fence))?;

        Ok(())
    }

    pub fn transfer_host_2d(
        &self,
        request: VirtioGpuTransferToHost2d,
    ) -> Result<(), VirtioGpuCommandError> {
        let (request_slice, response_slice) = Self::prepare_control_command(
            size_of::<VirtioGpuTransferToHost2d>(),
            size_of::<VirtioGpuCtrlHdr>(),
        )?;
        request_slice
            .write_val(0, &request)
            .map_err(VirtioGpuCommandError::ResourceAlloc)?;

        self.submit_control_command_async(&request_slice, &response_slice, None)?;
        Ok(())
    }

    pub fn transfer_host_3d(
        &self,
        request: VirtioGpuTransferHost3d,
        fence: Arc<DrmFence>,
    ) -> Result<(), VirtioGpuCommandError> {
        let (request_slice, response_slice) = Self::prepare_control_command(
            size_of::<VirtioGpuTransferHost3d>(),
            size_of::<VirtioGpuCtrlHdr>(),
        )?;

        request_slice
            .write_val(0, &request)
            .map_err(VirtioGpuCommandError::ResourceAlloc)?;

        self.submit_control_command_async(&request_slice, &response_slice, Some(fence))?;

        Ok(())
    }

    pub fn unref_resource(
        &self,
        request: VirtioGpuResourceUnref,
        gem_object: Arc<dyn DrmGemObject>,
    ) -> Result<(), VirtioGpuCommandError> {
        let (request_slice, response_slice) = Self::prepare_control_command(
            size_of::<VirtioGpuResourceUnref>(),
            size_of::<VirtioGpuCtrlHdr>(),
        )?;

        request_slice
            .write_val(0, &request)
            .map_err(VirtioGpuCommandError::ResourceAlloc)?;

        self.submit_control_command_async(&request_slice, &response_slice, None)?;

        Ok(())
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
