// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;
use core::{
    fmt,
    sync::atomic::{AtomicBool, Ordering},
};

use ostd::sync::{Mutex, WaitQueue, Waker};

bitflags::bitflags! {
    pub struct DrmSyncObjCreateFlags: u32 {
        const SIGNALED = 1 << 0;
    }
}

bitflags::bitflags! {
    pub struct DrmSyncObjWaitFlags: u32 {
        const WAIT_ALL = 1 << 0;
        const WAIT_FOR_SUBMIT = 1 << 1;
        const WAIT_AVAILABLE = 1 << 2;
        const WAIT_DEADLINE = 1 << 3;
    }
}

pub struct DrmFence {
    signaled: AtomicBool,
    wait_queue: WaitQueue,
}

impl fmt::Debug for DrmFence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DrmFence")
            .field("signaled", &self.is_signaled())
            .finish_non_exhaustive()
    }
}

impl DrmFence {
    pub fn new(signaled: bool) -> Self {
        Self {
            signaled: AtomicBool::new(signaled),
            wait_queue: WaitQueue::new(),
        }
    }

    pub fn is_signaled(&self) -> bool {
        self.signaled.load(Ordering::Acquire)
    }

    pub fn signal(&self) {
        if self.signaled.swap(true, Ordering::AcqRel) {
            return;
        }

        self.wait_queue.wake_all();
    }

    pub fn wait(&self) {
        self.wait_queue
            .wait_until(|| self.is_signaled().then_some(()));
    }

    pub fn register_waiter(&self, waker: Arc<Waker>) {
        self.wait_queue.enqueue(waker);
    }
}

pub struct DrmSyncObj {
    state: Mutex<DrmSyncObjState>,
    wait_queue: WaitQueue,
}

impl fmt::Debug for DrmSyncObj {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DrmSyncObj")
            .field("state", &self.state.lock())
            .finish_non_exhaustive()
    }
}

impl DrmSyncObj {
    pub fn new(signaled: bool) -> Self {
        Self {
            state: Mutex::new(DrmSyncObjState {
                fence: signaled.then(|| Arc::new(DrmFence::new(true))),
            }),
            wait_queue: WaitQueue::new(),
        }
    }

    pub fn is_submitted(&self) -> bool {
        self.state.lock().fence.is_some()
    }

    pub fn is_signaled(&self) -> bool {
        self.state
            .lock()
            .fence
            .as_ref()
            .is_some_and(|fence| fence.is_signaled())
    }

    pub fn wait(&self) {
        loop {
            let fence = self
                .wait_queue
                .wait_until(|| self.state.lock().fence.clone());
            fence.wait();
            if self.is_signaled() {
                return;
            }
        }
    }

    pub fn reset(&self) {
        self.state.lock().fence = None;
        self.wait_queue.wake_all();
    }

    pub fn signal(&self) {
        let fence = {
            let mut state = self.state.lock();
            state
                .fence
                .get_or_insert_with(|| Arc::new(DrmFence::new(false)))
                .clone()
        };

        fence.signal();
        self.wait_queue.wake_all();
    }

    pub fn set_fence(&self, fence: Arc<DrmFence>) {
        self.state.lock().fence = Some(fence);
        self.wait_queue.wake_all();
    }

    pub fn register_waiter(&self, waker: Arc<Waker>) {
        self.wait_queue.enqueue(waker.clone());

        if let Some(fence) = self.state.lock().fence.as_ref() {
            fence.register_waiter(waker);
        }
    }
}

#[derive(Debug, Default)]
struct DrmSyncObjState {
    fence: Option<Arc<DrmFence>>,
}
