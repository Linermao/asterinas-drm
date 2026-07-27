// SPDX-License-Identifier: MPL-2.0

use alloc::{
    boxed::Box,
    collections::VecDeque,
    fmt,
    sync::{Arc, Weak},
    vec,
    vec::Vec,
};
use core::{
    hint, mem,
    sync::atomic::{AtomicBool, Ordering},
};

use aster_drm::{DrmEdid, DrmError, DrmFence, DrmGemObject, DrmTaskSpawner};
use aster_softirq::BottomHalfDisabled;
use aster_util::mem_obj_slice::Slice;
use hashbrown::HashMap;
use ostd::{
    mm::{HasSize, PAGE_SIZE, VmIo, dma::DmaStream},
    sync::{Mutex, SpinLock, WaitQueue, Waiter, Waker},
};
use spin::Once;

use crate::{
    device::gpu::{
        VirtioGpuDeviceError,
        device::GpuDevice,
        gem::VirtioGpuHostVisibleAllocation,
        queue::{VirtioGpuCommandError, header::*},
    },
    queue::{AddBufsError, VirtQueue},
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
    fence: Option<(u64, Arc<DrmFence>)>,
    keepalive: Option<PendingCommandKeepalive>,
}

enum PendingCommandKeepalive {
    GemObject(Arc<dyn DrmGemObject>),
    HostVisibleAllocation(Arc<VirtioGpuHostVisibleAllocation>),
}

impl fmt::Debug for PendingCommandKeepalive {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GemObject(gem_object) => formatter
                .debug_tuple("GemObject")
                .field(gem_object)
                .finish(),
            Self::HostVisibleAllocation(allocation) => formatter
                .debug_tuple("HostVisibleAllocation")
                .field(allocation)
                .finish(),
        }
    }
}

struct ControlQueueState {
    queue: VirtQueue,
    pending_commands: HashMap<u16, PendingCommand>,
    deferred_commands: VecDeque<PendingCommand>,
}

pub(in crate::device::gpu) struct ControlQueueCallbackSignal {
    pending: AtomicBool,
    stopped: AtomicBool,
    wait_queue: WaitQueue,
}

impl ControlQueueCallbackSignal {
    fn new() -> Self {
        Self {
            pending: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
            wait_queue: WaitQueue::new(),
        }
    }

    pub(in crate::device::gpu) fn schedule(&self) {
        self.pending.store(true, Ordering::Release);
        self.wait_queue.wake_one();
    }

    fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
        self.wait_queue.wake_one();
    }

    fn wait(&self) -> bool {
        self.wait_queue.wait_until(|| {
            if self.stopped.load(Ordering::Acquire) {
                return Some(false);
            }

            self.pending.swap(false, Ordering::AcqRel).then_some(true)
        })
    }
}

pub struct ControlQueueManager {
    state: SpinLock<ControlQueueState, BottomHalfDisabled>,
    next_fence_id: Mutex<u64>,
    weak_device: Weak<GpuDevice>,
    callback_signal: Arc<ControlQueueCallbackSignal>,
    task_spawner: Once<Arc<dyn DrmTaskSpawner>>,
    callback_task: Once<()>,
}

impl ControlQueueManager {
    pub fn new(queue: VirtQueue, weak_device: &Weak<GpuDevice>) -> Self {
        Self {
            state: SpinLock::new(ControlQueueState {
                queue,
                pending_commands: HashMap::new(),
                deferred_commands: VecDeque::new(),
            }),
            next_fence_id: Mutex::new(1),
            weak_device: weak_device.clone(),
            callback_signal: Arc::new(ControlQueueCallbackSignal::new()),
            task_spawner: Once::new(),
            callback_task: Once::new(),
        }
    }

    pub(in crate::device::gpu) fn callback_signal(&self) -> Arc<ControlQueueCallbackSignal> {
        self.callback_signal.clone()
    }

    pub(in crate::device::gpu) fn init_task_context(&self, task_spawner: Arc<dyn DrmTaskSpawner>) {
        self.task_spawner.call_once(|| task_spawner);
    }

    fn ensure_callback_task(&self) -> Result<(), VirtioGpuCommandError> {
        let task_spawner = self
            .task_spawner
            .get()
            .ok_or(VirtioGpuCommandError::QueueUnavailable)?;
        self.callback_task.call_once(|| {
            // Component initialization happens before the first kernel thread.
            // The kernel supplies this spawner later, and the first normal
            // command starts the worker lazily.
            task_spawner.spawn(Box::new({
                let weak_device = self.weak_device.clone();
                let callback_signal = self.callback_signal.clone();
                move || {
                    while callback_signal.wait() {
                        let Some(device) = weak_device.upgrade() else {
                            break;
                        };
                        device.handle_control_queue_callback();
                    }
                }
            }));
        });
        Ok(())
    }
}

impl fmt::Debug for ControlQueueManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.state.lock();
        f.debug_struct("ControlQueueManager")
            .field("queue", &state.queue)
            .field("pending_command_count", &state.pending_commands.len())
            .field("deferred_command_count", &state.deferred_commands.len())
            .field(
                "task_context_initialized",
                &self.task_spawner.get().is_some(),
            )
            .field(
                "callback_task_initialized",
                &self.callback_task.get().is_some(),
            )
            .finish()
    }
}

impl Drop for ControlQueueManager {
    fn drop(&mut self) {
        self.callback_signal.stop();
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
        keepalive: Option<PendingCommandKeepalive>,
    ) -> Result<(), VirtioGpuCommandError> {
        let queue_manager = self.control_queue_manager();
        if let Err(error) = queue_manager.ensure_callback_task() {
            if let Some(keepalive) = keepalive {
                mem::forget(keepalive);
            }
            return Err(error);
        }

        if let Err(error) = request_slice.sync_to_device() {
            if let Some(keepalive) = keepalive {
                mem::forget(keepalive);
            }
            return Err(VirtioGpuCommandError::ResourceAlloc(error));
        }
        if let Err(error) = response_slice.sync_to_device() {
            if let Some(keepalive) = keepalive {
                mem::forget(keepalive);
            }
            return Err(VirtioGpuCommandError::ResourceAlloc(error));
        }

        // Hold this across preparation and insertion so fence ID order follows
        // fenced command order, without performing DMA synchronization under the
        // control-queue spin lock.
        let mut fence_guard = queue_manager.next_fence_id.lock();

        let pending_fence = if let Some(fence) = fence {
            let next_fence_id = *fence_guard;

            let mut request_header = match request_slice.read_val::<VirtioGpuCtrlHdr>(0) {
                Ok(request_header) => request_header,
                Err(error) => {
                    if let Some(keepalive) = keepalive {
                        mem::forget(keepalive);
                    }
                    return Err(VirtioGpuCommandError::ResourceAlloc(error));
                }
            };
            request_header.flags |= VirtioGpuFlags::FENCE.bits();
            request_header.fence_id = next_fence_id;
            if let Err(error) = request_slice.write_val(0, &request_header) {
                if let Some(keepalive) = keepalive {
                    mem::forget(keepalive);
                }
                return Err(VirtioGpuCommandError::ResourceAlloc(error));
            }
            if let Err(error) = request_slice.sync_to_device() {
                if let Some(keepalive) = keepalive {
                    mem::forget(keepalive);
                }
                return Err(VirtioGpuCommandError::ResourceAlloc(error));
            }

            *fence_guard = next_fence_id
                .checked_add(1)
                .ok_or(VirtioGpuCommandError::InvalidValue)?;

            Some((next_fence_id, fence))
        } else {
            None
        };

        let pending_command = PendingCommand {
            request_slice: request_slice.slice(0..request_slice.size()),
            response_slice: response_slice.slice(0..response_slice.size()),
            completion: PendingControlCompletion::Async,
            fence: pending_fence,
            keepalive,
        };

        Self::enqueue_control_command(queue_manager, pending_command);

        Ok(())
    }

    fn submit_control_command_sync(
        &self,
        request_slice: &Slice<Arc<DmaStream>>,
        response_slice: &Slice<Arc<DmaStream>>,
        expected_type: VirtioGpuCtrlType,
    ) -> Result<(), VirtioGpuCommandError> {
        let queue_manager = self.control_queue_manager();
        queue_manager.ensure_callback_task()?;

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
            fence: None,
            keepalive: None,
        };

        Self::enqueue_control_command(queue_manager, pending_command);

        waiter.wait();
        response_slice
            .sync_from_device()
            .map_err(VirtioGpuCommandError::ResourceAlloc)?;

        check_response_type(&response_slice, expected_type)?;

        Ok(())
    }

    fn enqueue_control_command(
        queue_manager: &ControlQueueManager,
        pending_command: PendingCommand,
    ) {
        let mut state = queue_manager.state.lock();
        if !state.deferred_commands.is_empty() {
            // Never let a newer command bypass the software queue. Virtio-gpu
            // resource creation, backing attachment, rendering, and cleanup all
            // rely on control-queue submission order.
            state.deferred_commands.push_back(pending_command);
            return;
        }

        let pending_command = match Self::try_add_control_command(&mut state, pending_command) {
            Ok(()) => {
                if state.queue.should_notify() {
                    state.queue.notify();
                }
                return;
            }
            Err(pending_command) => pending_command,
        };
        state.deferred_commands.push_back(pending_command);
    }

    fn try_add_control_command(
        state: &mut ControlQueueState,
        pending_command: PendingCommand,
    ) -> Result<(), PendingCommand> {
        let result = state.queue.add_dma_bufs(
            &[&pending_command.request_slice],
            &[&pending_command.response_slice],
        );
        match result {
            Ok(token) => {
                state.pending_commands.insert(token, pending_command);
                Ok(())
            }
            Err(AddBufsError::BufferTooSmall) => Err(pending_command),
            Err(AddBufsError::InvalidArgs) => {
                unreachable!("a control command always has one input and one output buffer")
            }
        }
    }

    fn submit_deferred_commands(state: &mut ControlQueueState) -> bool {
        let mut submitted = false;
        while let Some(pending_command) = state.deferred_commands.pop_front() {
            match Self::try_add_control_command(state, pending_command) {
                Ok(()) => submitted = true,
                Err(pending_command) => {
                    state.deferred_commands.push_front(pending_command);
                    break;
                }
            }
        }
        submitted
    }

    fn handle_control_queue_callback(&self) {
        let queue_manager = self.control_queue_manager();
        let completed_commands = {
            let mut state = queue_manager.state.lock();
            let mut completed_commands = Vec::new();

            while let Ok((token, _)) = state
                .queue
                .pop_used_with_min_bytes(size_of::<VirtioGpuCtrlHdr>())
            {
                let Some(pending_command) = state.pending_commands.remove(&token) else {
                    ostd::warn!("virtio-gpu completed unknown control command");
                    continue;
                };
                completed_commands.push(pending_command);
            }

            if Self::submit_deferred_commands(&mut state) && state.queue.should_notify() {
                state.queue.notify();
            }

            completed_commands
        };

        for pending_command in completed_commands {
            // DMA synchronization, waking tasks, and releasing cleanup
            // keepalives are intentionally done in task context.
            self.complete_control_command(pending_command);
        }
    }

    fn complete_control_command(&self, pending_command: PendingCommand) {
        let PendingCommand {
            request_slice,
            response_slice,
            completion,
            fence,
            mut keepalive,
        } = pending_command;

        match completion {
            PendingControlCompletion::Sync(waker) => {
                waker.wake_up();
            }
            PendingControlCompletion::Async => {
                let response_result = response_slice
                    .sync_from_device()
                    .map_err(VirtioGpuCommandError::ResourceAlloc)
                    .and_then(|()| {
                        check_response_type(&response_slice, VirtioGpuCtrlType::RespOkNodata)
                    });
                let request_type = request_slice
                    .read_val::<VirtioGpuCtrlHdr>(0)
                    .ok()
                    .and_then(|header| VirtioGpuCtrlType::try_from(header.type_).ok())
                    .unwrap_or(VirtioGpuCtrlType::Unknown);

                if let Err(error) = &response_result {
                    ostd::warn!(
                        "virtio-gpu async control command failed: request={:?}, error={:?}",
                        request_type,
                        error
                    );
                    if let Some(keepalive) = keepalive.take() {
                        // A failed UNREF or UNMAP does not permit releasing the
                        // memory that may still be visible to the host.
                        mem::forget(keepalive);
                    }
                }

                if let Some((expected_fence_id, fence)) = fence {
                    let completion_result = response_result
                        .map_err(DrmError::from)
                        .and_then(|response_header| {
                            let has_fence =
                                response_header.flags & VirtioGpuFlags::FENCE.bits() != 0;
                            if !has_fence || response_header.fence_id != expected_fence_id {
                                ostd::warn!(
                                    "virtio-gpu returned an invalid fence response: request={:?}, expected_fence_id={}, response_fence_id={}, response_flags={:#x}",
                                    request_type,
                                    expected_fence_id,
                                    response_header.fence_id,
                                    response_header.flags
                                );
                                return Err(DrmError::Invalid);
                            }
                            Ok(())
                        });

                    match completion_result {
                        Ok(()) => {
                            fence.signal();
                        }
                        Err(error) => {
                            fence.signal_error(error);
                        }
                    }
                }
            }
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

        self.submit_control_command_async(&request_slice, &response_slice, None, None)?;

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

        self.submit_control_command_async(&request_slice, &response_slice, None, None)?;

        Ok(())
    }

    pub fn destroy_context(
        &self,
        request: VirtioGpuCtxDestroy,
    ) -> Result<(), VirtioGpuCommandError> {
        let (request_slice, response_slice) = Self::prepare_control_command(
            size_of::<VirtioGpuCtxDestroy>(),
            size_of::<VirtioGpuCtrlHdr>(),
        )?;

        request_slice
            .write_val(0, &request)
            .map_err(VirtioGpuCommandError::ResourceAlloc)?;

        self.submit_control_command_async(&request_slice, &response_slice, None, None)?;

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

        self.submit_control_command_async(&request_slice, &response_slice, Some(fence), None)?;

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

        self.submit_control_command_async(
            &request_slice,
            &response_slice,
            Some(fence.clone()),
            None,
        )?;

        Ok(())
    }

    pub fn create_blob_resource(
        &self,
        request: VirtioGpuResourceCreateBlob,
        entries: &[VirtioGpuMemEntry],
        fence: Arc<DrmFence>,
    ) -> Result<(), VirtioGpuCommandError> {
        let entries_size = entries
            .len()
            .checked_mul(size_of::<VirtioGpuMemEntry>())
            .ok_or(VirtioGpuCommandError::InvalidValue)?;
        let request_size = size_of::<VirtioGpuResourceCreateBlob>()
            .checked_add(entries_size)
            .ok_or(VirtioGpuCommandError::InvalidValue)?;
        let (request_slice, response_slice) =
            Self::prepare_control_command(request_size, size_of::<VirtioGpuCtrlHdr>())?;

        request_slice
            .write_val(0, &request)
            .map_err(VirtioGpuCommandError::ResourceAlloc)?;
        request_slice
            .write_slice(size_of::<VirtioGpuResourceCreateBlob>(), entries)
            .map_err(VirtioGpuCommandError::ResourceAlloc)?;

        self.submit_control_command_async(&request_slice, &response_slice, Some(fence), None)?;

        Ok(())
    }

    pub fn map_blob_resource(
        &self,
        request: VirtioGpuResourceMapBlob,
    ) -> Result<u32, VirtioGpuCommandError> {
        let (request_slice, response_slice) = Self::prepare_control_command(
            size_of::<VirtioGpuResourceMapBlob>(),
            size_of::<VirtioGpuRespMapInfo>(),
        )?;

        request_slice
            .write_val(0, &request)
            .map_err(VirtioGpuCommandError::ResourceAlloc)?;

        self.submit_control_command_sync(
            &request_slice,
            &response_slice,
            VirtioGpuCtrlType::RespOkMapInfo,
        )?;

        let response = response_slice
            .read_val::<VirtioGpuRespMapInfo>(0)
            .map_err(VirtioGpuCommandError::ResourceAlloc)?;
        Ok(response.map_info)
    }

    pub(in crate::device::gpu) fn unmap_blob_resource(
        &self,
        request: VirtioGpuResourceUnmapBlob,
        allocation: Arc<VirtioGpuHostVisibleAllocation>,
    ) -> Result<(), VirtioGpuCommandError> {
        let keepalive = PendingCommandKeepalive::HostVisibleAllocation(allocation);
        let (request_slice, response_slice) = match Self::prepare_control_command(
            size_of::<VirtioGpuResourceUnmapBlob>(),
            size_of::<VirtioGpuCtrlHdr>(),
        ) {
            Ok(slices) => slices,
            Err(error) => {
                mem::forget(keepalive);
                return Err(error);
            }
        };

        if let Err(error) = request_slice.write_val(0, &request) {
            mem::forget(keepalive);
            return Err(VirtioGpuCommandError::ResourceAlloc(error));
        }

        self.submit_control_command_async(&request_slice, &response_slice, None, Some(keepalive))
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

        self.submit_control_command_async(&request_slice, &response_slice, None, None)?;

        Ok(())
    }

    pub fn detach_backing_sg_entries(
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

        self.submit_control_command_async(&request_slice, &response_slice, None, None)?;

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
        fence: Option<Arc<DrmFence>>,
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

        self.submit_control_command_async(&request_slice, &response_slice, fence, None)?;

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

        self.submit_control_command_async(&request_slice, &response_slice, None, None)?;
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

        self.submit_control_command_async(&request_slice, &response_slice, Some(fence), None)?;

        Ok(())
    }

    pub(in crate::device::gpu) fn unref_resource(
        &self,
        request: VirtioGpuResourceUnref,
        gem_object: Arc<dyn DrmGemObject>,
    ) -> Result<(), VirtioGpuCommandError> {
        let keepalive = PendingCommandKeepalive::GemObject(gem_object);
        let (request_slice, response_slice) = match Self::prepare_control_command(
            size_of::<VirtioGpuResourceUnref>(),
            size_of::<VirtioGpuCtrlHdr>(),
        ) {
            Ok(slices) => slices,
            Err(error) => {
                mem::forget(keepalive);
                return Err(error);
            }
        };

        if let Err(error) = request_slice.write_val(0, &request) {
            mem::forget(keepalive);
            return Err(VirtioGpuCommandError::ResourceAlloc(error));
        }

        self.submit_control_command_async(&request_slice, &response_slice, None, Some(keepalive))?;

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
