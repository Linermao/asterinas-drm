// SPDX-License-Identifier: MPL-2.0

use alloc::{
    sync::{Arc, Weak},
    vec::Vec,
};
use core::{
    fmt,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::Duration,
};

use aster_time::{Timeout, monotonic_timer_manager};
use ostd::sync::{LocalIrqDisabled, SpinLock, WaitQueue, Waiter, Waker};

use crate::DrmError;

static NEXT_FENCE_CONTEXT: AtomicU64 = AtomicU64::new(1);

/// Describes the completion state of a DRM fence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DrmFenceStatus {
    /// The operation represented by the fence has not completed.
    Pending,
    /// The operation represented by the fence completed successfully.
    Completed,
    /// The operation represented by the fence completed with an error.
    Failed(DrmError),
}

/// Represents the completion of an asynchronous DRM operation.
///
/// A fence is either a native fence signaled by a driver, or a fence-chain
/// node. A chain node represents both its contained fence and every preceding
/// fence, matching the cumulative completion semantics of Linux fence chains.
pub struct DrmFence {
    kind: DrmFenceKind,
}

enum DrmFenceKind {
    Native(DrmNativeFence),
    Chain(DrmFenceChain),
}

struct DrmNativeFence {
    state: SpinLock<DrmNativeFenceState, LocalIrqDisabled>,
    wait_queue: WaitQueue,
}

struct DrmNativeFenceState {
    status: DrmFenceStatus,
    callbacks: Vec<Arc<dyn DrmFenceCallback>>,
}

struct DrmFenceChain {
    context: u64,
    sequence_number: u64,
    previous_sequence_number: u64,
    contained: Arc<DrmFence>,
    // The link is mutable so that completed prefixes can be pruned later
    // without replacing the chain head held by users of this fence.
    previous: SpinLock<Option<Arc<DrmFence>>, LocalIrqDisabled>,
}

/// Receives a one-shot notification after a DRM fence completes.
///
/// Implementations can run in an interrupt or bottom-half context and must not
/// block. The callback is always invoked after the fence's internal lock has
/// been released.
pub trait DrmFenceCallback: Send + Sync {
    /// Handles completion of the registered fence.
    fn on_signal(&self, status: DrmFenceStatus);
}

impl<F> DrmFenceCallback for F
where
    F: Fn(DrmFenceStatus) + Send + Sync,
{
    fn on_signal(&self, status: DrmFenceStatus) {
        self(status);
    }
}

struct DrmFenceChainCallback {
    fence: Weak<DrmFence>,
    callback: Arc<dyn DrmFenceCallback>,
    called: AtomicBool,
}

impl DrmFenceChainCallback {
    fn notify_if_signaled(&self) {
        if self.called.load(Ordering::Acquire) {
            return;
        }

        let Some(fence) = self.fence.upgrade() else {
            return;
        };
        let status = fence.status();
        if status == DrmFenceStatus::Pending {
            return;
        }

        if !self.called.swap(true, Ordering::AcqRel) {
            self.callback.on_signal(status);
        }
    }
}

impl fmt::Debug for DrmFence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = formatter.debug_struct("DrmFence");
        debug.field("status", &self.status());

        match &self.kind {
            DrmFenceKind::Native(_) => {
                debug.field("kind", &"native");
            }
            DrmFenceKind::Chain(chain) => {
                debug
                    .field("kind", &"chain")
                    .field("context", &chain.context)
                    .field("sequence_number", &chain.sequence_number)
                    .field("previous_sequence_number", &chain.previous_sequence_number);
            }
        }

        debug.finish_non_exhaustive()
    }
}

impl DrmFence {
    /// Creates a native fence in the requested initial completion state.
    pub fn new(signaled: bool) -> Self {
        let status = if signaled {
            DrmFenceStatus::Completed
        } else {
            DrmFenceStatus::Pending
        };

        Self {
            kind: DrmFenceKind::Native(DrmNativeFence {
                state: SpinLock::new(DrmNativeFenceState {
                    status,
                    callbacks: Vec::new(),
                }),
                wait_queue: WaitQueue::new(),
            }),
        }
    }

    /// Creates a fence-chain node for a timeline sequence number.
    ///
    /// The returned fence becomes signaled only after `contained` and every
    /// fence reachable through `previous` have signaled. The sequence number
    /// is metadata used to find the chain node covering a timeline point; it
    /// does not by itself signal preceding work.
    pub fn new_chain(
        previous: Option<Arc<DrmFence>>,
        contained: Arc<DrmFence>,
        sequence_number: u64,
    ) -> Result<Self, DrmError> {
        if matches!(&contained.kind, DrmFenceKind::Chain(_)) {
            return Err(DrmError::Invalid);
        }

        let previous_chain = previous.as_ref().and_then(|fence| match &fence.kind {
            DrmFenceKind::Native(_) => None,
            DrmFenceKind::Chain(chain) => Some(chain),
        });
        let (context, sequence_number, previous_sequence_number) = match previous_chain {
            Some(previous_chain) if sequence_number > previous_chain.sequence_number => (
                previous_chain.context,
                sequence_number,
                previous_chain.sequence_number,
            ),
            Some(previous_chain) => (
                Self::allocate_context(),
                sequence_number.max(previous_chain.sequence_number),
                0,
            ),
            None => (Self::allocate_context(), sequence_number, 0),
        };

        Ok(Self {
            kind: DrmFenceKind::Chain(DrmFenceChain {
                context,
                sequence_number,
                previous_sequence_number,
                contained,
                previous: SpinLock::new(previous),
            }),
        })
    }

    /// Returns the current completion state of the fence.
    pub fn status(&self) -> DrmFenceStatus {
        match &self.kind {
            DrmFenceKind::Native(native) => native.state.lock().status.clone(),
            DrmFenceKind::Chain(_) => self.chain_status(),
        }
    }

    /// Returns the completion error after the fence has signaled with one.
    pub fn error(&self) -> Option<DrmError> {
        match self.status() {
            DrmFenceStatus::Failed(error) => Some(error),
            DrmFenceStatus::Pending | DrmFenceStatus::Completed => None,
        }
    }

    /// Returns whether the fence has completed, successfully or otherwise.
    pub fn is_signaled(&self) -> bool {
        !matches!(self.status(), DrmFenceStatus::Pending)
    }

    /// Signals a native fence successfully.
    ///
    /// Returns `true` when this call performed the pending-to-completed
    /// transition. Derived chain fences cannot be signaled directly.
    pub fn signal(&self) -> bool {
        self.complete_native(DrmFenceStatus::Completed)
    }

    /// Signals a native fence with an operation error.
    ///
    /// Returns `true` when this call performed the pending-to-failed
    /// transition. The error is installed before waiters are woken.
    pub fn signal_error(&self, error: DrmError) -> bool {
        self.complete_native(DrmFenceStatus::Failed(error))
    }

    /// Returns the timeline sequence number carried by a chain fence.
    pub fn sequence_number(&self) -> Option<u64> {
        match &self.kind {
            DrmFenceKind::Native(_) => None,
            DrmFenceKind::Chain(chain) => Some(chain.sequence_number),
        }
    }

    /// Returns the original predecessor sequence number of a chain node.
    pub fn previous_sequence_number(&self) -> Option<u64> {
        match &self.kind {
            DrmFenceKind::Native(_) => None,
            DrmFenceKind::Chain(chain) => Some(chain.previous_sequence_number),
        }
    }

    /// Returns whether two chain fences belong to the same ordered timeline.
    ///
    /// An unordered point starts a new context, so this method also provides
    /// the discontinuity check needed by future syncobj query operations.
    pub fn is_same_timeline(&self, other: &DrmFence) -> bool {
        match (&self.kind, &other.kind) {
            (DrmFenceKind::Chain(this), DrmFenceKind::Chain(other)) => {
                this.context == other.context
            }
            _ => false,
        }
    }

    /// Returns the fence contained by this chain node.
    pub fn contained_fence(&self) -> Option<Arc<DrmFence>> {
        match &self.kind {
            DrmFenceKind::Native(_) => None,
            DrmFenceKind::Chain(chain) => Some(chain.contained.clone()),
        }
    }

    /// Returns the preceding fence of this chain node.
    pub fn previous_fence(&self) -> Option<Arc<DrmFence>> {
        match &self.kind {
            DrmFenceKind::Native(_) => None,
            DrmFenceKind::Chain(chain) => chain.previous.lock().clone(),
        }
    }

    /// Finds the chain fence that covers the requested timeline point.
    ///
    /// Point zero returns the supplied fence unchanged, preserving binary
    /// syncobj behavior. For nonzero points this follows the same interval
    /// rule as Linux fence-chain lookup: a later chain node also represents
    /// all points after its predecessor and through its own sequence number.
    pub fn find_chain_point(self: &Arc<Self>, point: u64) -> Option<Arc<Self>> {
        if point == 0 {
            return Some(self.clone());
        }

        let head_sequence_number = self.sequence_number()?;
        if head_sequence_number < point {
            return None;
        }
        let DrmFenceKind::Chain(head_chain) = &self.kind else {
            return None;
        };
        let head_context = head_chain.context;

        let mut current = self.clone();
        loop {
            let DrmFenceKind::Chain(chain) = &current.kind else {
                return None;
            };

            if chain.context != head_context || chain.previous_sequence_number < point {
                return Some(current);
            }

            let previous = chain.previous.lock().clone();
            let Some(previous) = previous else {
                return Some(current);
            };
            current = previous;
        }
    }

    /// Waits until the fence completes.
    pub fn wait(&self) {
        if self.is_signaled() {
            return;
        }

        let (waiter, _) = Waiter::new_pair();
        waiter
            .wait_until_or_cancelled(
                || {
                    self.register_waiter(waiter.waker());
                    self.is_signaled().then_some(())
                },
                || Ok::<(), ()>(()),
            )
            .unwrap();
    }

    /// Waits until the fence completes or the relative timeout expires.
    pub fn wait_timeout(&self, timeout: Option<Duration>) -> Result<(), DrmError> {
        let deadline = timeout.map(|duration| {
            monotonic_timer_manager()
                .read_time()
                .saturating_add(duration)
        });
        self.wait_deadline(deadline)
    }

    pub(super) fn wait_deadline(&self, deadline: Option<Duration>) -> Result<(), DrmError> {
        if self.is_signaled() {
            return self.completion_result();
        }
        if deadline.is_some_and(|deadline| monotonic_timer_manager().read_time() >= deadline) {
            return Err(DrmError::Busy);
        }

        let (waiter, _) = Waiter::new_pair();
        let timer = deadline.map(|deadline| {
            let waker = waiter.waker();
            let timer = monotonic_timer_manager().create_timer(move |_guard| {
                waker.wake_up();
            });
            timer.lock().set_timeout(Timeout::When(deadline));
            timer
        });

        let result = waiter.wait_until_or_cancelled(
            || {
                self.register_waiter(waiter.waker());
                self.is_signaled().then_some(())
            },
            || {
                if timer
                    .as_ref()
                    .is_some_and(|timer| timer.lock().remain() == Duration::ZERO)
                {
                    return Err(DrmError::Busy);
                }

                Ok(())
            },
        );

        if let Some(timer) = timer
            && result != Err(DrmError::Busy)
        {
            timer.lock().cancel();
        }

        result?;
        self.completion_result()
    }

    /// Registers a waiter on every pending native fence represented by this fence.
    ///
    /// Returns `true` if at least one pending fence accepted the waiter. Native
    /// registration and signaling are serialized so a completed fence never
    /// retains a waiter that cannot be woken.
    pub fn register_waiter(&self, waker: Arc<Waker>) -> bool {
        let mut pending_fences = Vec::new();
        let mut registered = false;

        match &self.kind {
            DrmFenceKind::Native(native) => {
                return Self::register_native_waiter(native, waker);
            }
            DrmFenceKind::Chain(chain) => {
                pending_fences.push(chain.contained.clone());
                if let Some(previous) = chain.previous.lock().clone() {
                    pending_fences.push(previous);
                }
            }
        }

        while let Some(fence) = pending_fences.pop() {
            match &fence.kind {
                DrmFenceKind::Native(native) => {
                    registered |= Self::register_native_waiter(native, waker.clone());
                }
                DrmFenceKind::Chain(chain) => {
                    pending_fences.push(chain.contained.clone());
                    if let Some(previous) = chain.previous.lock().clone() {
                        pending_fences.push(previous);
                    }
                }
            }
        }

        registered
    }

    /// Adds a one-shot callback to a pending fence.
    ///
    /// Returns `false` without invoking the callback if the fence had already
    /// completed. A callback accepted by a chain fence runs only after every
    /// fence represented by that chain has completed.
    pub fn add_callback(self: &Arc<Self>, callback: Arc<dyn DrmFenceCallback>) -> bool {
        match &self.kind {
            DrmFenceKind::Native(native) => Self::add_native_callback(native, callback),
            DrmFenceKind::Chain(_) => self.add_chain_callback(callback),
        }
    }

    /// Returns a native fence suitable for containment in another chain.
    ///
    /// Linux does not permit a fence chain as the contained fence of another
    /// chain. A native fence is returned unchanged, while a chain is flattened
    /// into a proxy that mirrors its cumulative completion status.
    pub fn as_chain_dependency(self: &Arc<Self>) -> Arc<Self> {
        if matches!(&self.kind, DrmFenceKind::Native(_)) {
            return self.clone();
        }

        let dependency = Arc::new(Self::new(false));
        let callback_dependency = dependency.clone();
        let accepted = self.add_callback(Arc::new(move |status| match status {
            DrmFenceStatus::Pending => {}
            DrmFenceStatus::Completed => {
                callback_dependency.signal();
            }
            DrmFenceStatus::Failed(error) => {
                callback_dependency.signal_error(error);
            }
        }));

        if !accepted {
            match self.status() {
                DrmFenceStatus::Pending => {}
                DrmFenceStatus::Completed => {
                    dependency.signal();
                }
                DrmFenceStatus::Failed(error) => {
                    dependency.signal_error(error);
                }
            }
        }

        dependency
    }

    fn register_native_waiter(native: &DrmNativeFence, waker: Arc<Waker>) -> bool {
        let state = native.state.lock();
        if state.status != DrmFenceStatus::Pending {
            return false;
        }

        native.wait_queue.enqueue(waker);
        true
    }

    fn add_native_callback(native: &DrmNativeFence, callback: Arc<dyn DrmFenceCallback>) -> bool {
        let mut state = native.state.lock();
        if state.status != DrmFenceStatus::Pending {
            return false;
        }

        state.callbacks.push(callback);
        true
    }

    fn add_chain_callback(self: &Arc<Self>, callback: Arc<dyn DrmFenceCallback>) -> bool {
        if self.is_signaled() {
            return false;
        }

        let callback_state = Arc::new(DrmFenceChainCallback {
            fence: Arc::downgrade(self),
            callback,
            called: AtomicBool::new(false),
        });
        let callback_fn: Arc<dyn DrmFenceCallback> = {
            let callback_state = callback_state.clone();
            Arc::new(move |_status| callback_state.notify_if_signaled())
        };

        let mut fences = Vec::from([self.clone()]);
        let mut registered = false;
        while let Some(fence) = fences.pop() {
            match &fence.kind {
                DrmFenceKind::Native(native) => {
                    registered |= Self::add_native_callback(native, callback_fn.clone());
                }
                DrmFenceKind::Chain(chain) => {
                    fences.push(chain.contained.clone());
                    if let Some(previous) = chain.previous.lock().clone() {
                        fences.push(previous);
                    }
                }
            }
        }

        if registered {
            callback_state.notify_if_signaled();
        }
        registered
    }

    fn complete_native(&self, completed_status: DrmFenceStatus) -> bool {
        let DrmFenceKind::Native(native) = &self.kind else {
            return false;
        };

        let mut state = native.state.lock();
        if state.status != DrmFenceStatus::Pending {
            return false;
        }

        state.status = completed_status.clone();
        let callbacks = core::mem::take(&mut state.callbacks);
        drop(state);
        native.wait_queue.wake_all();
        for callback in callbacks {
            callback.on_signal(completed_status.clone());
        }
        true
    }

    fn chain_status(&self) -> DrmFenceStatus {
        let DrmFenceKind::Chain(chain) = &self.kind else {
            unreachable!();
        };

        let mut fences = Vec::from([chain.contained.clone()]);
        if let Some(previous) = chain.previous.lock().clone() {
            fences.push(previous);
        }
        let mut first_error = None;

        while let Some(fence) = fences.pop() {
            match &fence.kind {
                DrmFenceKind::Native(native) => match native.state.lock().status.clone() {
                    DrmFenceStatus::Pending => return DrmFenceStatus::Pending,
                    DrmFenceStatus::Completed => {}
                    DrmFenceStatus::Failed(error) => {
                        first_error.get_or_insert(error);
                    }
                },
                DrmFenceKind::Chain(chain) => {
                    fences.push(chain.contained.clone());
                    if let Some(previous) = chain.previous.lock().clone() {
                        fences.push(previous);
                    }
                }
            }
        }

        first_error.map_or(DrmFenceStatus::Completed, DrmFenceStatus::Failed)
    }

    fn completion_result(&self) -> Result<(), DrmError> {
        match self.status() {
            DrmFenceStatus::Completed => Ok(()),
            DrmFenceStatus::Failed(error) => Err(error),
            DrmFenceStatus::Pending => Err(DrmError::Busy),
        }
    }

    fn allocate_context() -> u64 {
        NEXT_FENCE_CONTEXT
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |context| {
                context.checked_add(1)
            })
            .expect("exhausted DRM fence contexts")
    }
}

impl Drop for DrmFence {
    fn drop(&mut self) {
        let DrmFenceKind::Chain(chain) = &self.kind else {
            return;
        };

        // Detaches uniquely owned predecessors iteratively. Otherwise dropping
        // a long chain would recursively drop one `Arc` per timeline point.
        let mut previous = chain.previous.lock().take();
        while let Some(fence) = previous {
            if Arc::strong_count(&fence) != 1 {
                break;
            }

            previous = match &fence.kind {
                DrmFenceKind::Native(_) => None,
                DrmFenceKind::Chain(chain) => chain.previous.lock().take(),
            };
        }
    }
}

#[cfg(ktest)]
mod tests {
    use core::sync::atomic::AtomicUsize;

    use ostd::prelude::*;

    use super::*;

    #[ktest]
    fn native_fence_records_exactly_one_completion() {
        let fence = DrmFence::new(false);

        assert_eq!(fence.status(), DrmFenceStatus::Pending);
        assert!(fence.signal_error(DrmError::Invalid));
        assert_eq!(fence.status(), DrmFenceStatus::Failed(DrmError::Invalid));
        assert!(!fence.signal());
        assert_eq!(fence.error(), Some(DrmError::Invalid));
    }

    #[ktest]
    fn chain_completion_includes_all_preceding_fences() {
        let first = Arc::new(DrmFence::new(false));
        let first_point = Arc::new(DrmFence::new_chain(None, first.clone(), 10).unwrap());
        let second = Arc::new(DrmFence::new(true));
        let second_point = Arc::new(DrmFence::new_chain(Some(first_point), second, 20).unwrap());

        assert_eq!(second_point.status(), DrmFenceStatus::Pending);
        first.signal();
        assert_eq!(second_point.status(), DrmFenceStatus::Completed);
    }

    #[ktest]
    fn chain_lookup_uses_timeline_intervals() {
        let first_point =
            Arc::new(DrmFence::new_chain(None, Arc::new(DrmFence::new(false)), 10).unwrap());
        let second_point = Arc::new(
            DrmFence::new_chain(
                Some(first_point.clone()),
                Arc::new(DrmFence::new(false)),
                20,
            )
            .unwrap(),
        );

        assert!(Arc::ptr_eq(
            &second_point.find_chain_point(0).unwrap(),
            &second_point
        ));
        assert!(Arc::ptr_eq(
            &second_point.find_chain_point(10).unwrap(),
            &first_point
        ));
        assert!(Arc::ptr_eq(
            &second_point.find_chain_point(11).unwrap(),
            &second_point
        ));
        assert!(second_point.find_chain_point(21).is_none());
    }

    #[ktest]
    fn chain_callback_runs_once_after_all_dependencies() {
        let first = Arc::new(DrmFence::new(false));
        let first_point = Arc::new(DrmFence::new_chain(None, first.clone(), 10).unwrap());
        let second = Arc::new(DrmFence::new(false));
        let second_point =
            Arc::new(DrmFence::new_chain(Some(first_point), second.clone(), 20).unwrap());
        let callback_count = Arc::new(AtomicUsize::new(0));
        let callback_count_clone = callback_count.clone();

        assert!(second_point.add_callback(Arc::new(move |status| {
            assert_eq!(status, DrmFenceStatus::Completed);
            callback_count_clone.fetch_add(1, Ordering::Relaxed);
        })));
        second.signal();
        assert_eq!(callback_count.load(Ordering::Relaxed), 0);
        first.signal();
        assert_eq!(callback_count.load(Ordering::Relaxed), 1);
    }

    #[ktest]
    fn unordered_chain_starts_a_new_timeline_context() {
        let first_point =
            Arc::new(DrmFence::new_chain(None, Arc::new(DrmFence::new(false)), 10).unwrap());
        let unordered_point = Arc::new(
            DrmFence::new_chain(Some(first_point.clone()), Arc::new(DrmFence::new(false)), 5)
                .unwrap(),
        );

        assert_eq!(unordered_point.sequence_number(), Some(10));
        assert!(!unordered_point.is_same_timeline(&first_point));
        assert!(Arc::ptr_eq(
            &unordered_point.find_chain_point(10).unwrap(),
            &unordered_point
        ));
    }

    #[ktest]
    fn chain_rejects_another_chain_as_contained_fence() {
        let contained =
            Arc::new(DrmFence::new_chain(None, Arc::new(DrmFence::new(false)), 10).unwrap());

        assert!(DrmFence::new_chain(None, contained, 20).is_err());
    }
}
