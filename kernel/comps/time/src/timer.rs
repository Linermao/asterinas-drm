// SPDX-License-Identifier: MPL-2.0

use alloc::{
    boxed::Box,
    collections::BinaryHeap,
    sync::{Arc, Weak},
    vec::Vec,
};
use core::{
    cmp::Ordering as CmpOrdering,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use ostd::sync::{LocalIrqDisabled, SpinLock, SpinLockGuard};

/// A timeout, represented in one of two ways.
#[derive(Clone, Debug)]
pub enum Timeout {
    /// The timeout is reached after the duration has elapsed.
    After(Duration),
    /// The timeout is reached when monotonic time reaches the duration.
    When(Duration),
}

/// A timer managed by a [`TimerManager`].
pub struct Timer {
    inner: SpinLock<TimerInner>,
    timer_manager: Arc<TimerManager>,
    registered_callback: Box<dyn Fn(TimerGuard) + Send + Sync>,
}

#[derive(Default)]
struct TimerInner {
    interval: Duration,
    timer_callback: Weak<TimerCallback>,
}

/// A guard that provides exclusive access to a [`Timer`].
pub struct TimerGuard<'a> {
    inner: SpinLockGuard<'a, TimerInner, LocalIrqDisabled>,
    timer: &'a Arc<Timer>,
}

impl TimerGuard<'_> {
    /// Sets the interval time for this timer.
    pub fn set_interval(&mut self, interval: Duration) {
        self.inner.interval = interval;
    }

    /// Sets the timer with a timeout.
    pub fn set_timeout(&mut self, timeout: Timeout) {
        let expired_time = match timeout {
            Timeout::After(timeout) => self.timer.timer_manager.read_time() + timeout,
            Timeout::When(timeout) => timeout,
        };

        let timer_weak = Arc::downgrade(self.timer);
        let new_timer_callback = Arc::new(TimerCallback::new(expired_time, timer_weak));

        if let Some(timer_callback) = self.inner.timer_callback.upgrade() {
            timer_callback.cancel();
        }

        self.inner.timer_callback = Arc::downgrade(&new_timer_callback);
        self.timer.timer_manager.insert(new_timer_callback);
    }

    /// Cancels the currently set timer callback.
    pub fn cancel(&self) {
        if let Some(timer_callback) = self.inner.timer_callback.upgrade() {
            timer_callback.cancel();
        }
    }

    /// Returns the current expired time of this timer.
    pub fn expired_time(&self) -> Duration {
        let timer_callback = self.inner.timer_callback.upgrade();
        timer_callback
            .and_then(|callback| (!callback.is_cancelled()).then_some(callback.expired_time))
            .unwrap_or(Duration::ZERO)
    }

    /// Returns the remaining time to expiration.
    pub fn remain(&self) -> Duration {
        let now = self.timer.timer_manager.read_time();
        let expired_time = self.expired_time();
        if expired_time > now {
            expired_time - now
        } else {
            Duration::ZERO
        }
    }

    /// Returns the interval time of the current timer.
    pub fn interval(&self) -> Duration {
        self.inner.interval
    }
}

impl Timer {
    fn new<F>(registered_callback: F, timer_manager: Arc<TimerManager>) -> Arc<Self>
    where
        F: Fn(TimerGuard) + Send + Sync + 'static,
    {
        Arc::new(Self {
            inner: SpinLock::new(TimerInner::default()),
            timer_manager,
            registered_callback: Box::new(registered_callback),
        })
    }

    /// Locks the timer and returns a [`TimerGuard`].
    pub fn lock(self: &Arc<Self>) -> TimerGuard<'_> {
        TimerGuard {
            inner: self.inner.disable_irq().lock(),
            timer: self,
        }
    }

    /// Returns the [`TimerManager`] that manages this timer.
    pub fn timer_manager(&self) -> &Arc<TimerManager> {
        &self.timer_manager
    }
}

/// Manages timers based on a monotonic clock callback.
pub struct TimerManager {
    read_time: Arc<dyn Fn() -> Duration + Send + Sync>,
    timer_callbacks: SpinLock<BinaryHeap<Arc<TimerCallback>>>,
}

impl TimerManager {
    /// Creates a timer manager.
    pub fn new(read_time: Arc<dyn Fn() -> Duration + Send + Sync>) -> Arc<Self> {
        Arc::new(Self {
            read_time,
            timer_callbacks: SpinLock::new(BinaryHeap::new()),
        })
    }

    /// Reads the current time.
    pub fn read_time(&self) -> Duration {
        (self.read_time)()
    }

    /// Returns whether a timeout is expired.
    pub fn is_expired_timeout(&self, timeout: &Timeout) -> bool {
        match timeout {
            Timeout::After(duration) => *duration == Duration::ZERO,
            Timeout::When(duration) => self.read_time() >= *duration,
        }
    }

    fn insert(&self, timer_callback: Arc<TimerCallback>) {
        self.timer_callbacks
            .disable_irq()
            .lock()
            .push(timer_callback);
    }

    /// Checks and processes expired timers.
    pub fn process_expired_timers(&self) {
        let callbacks = {
            let mut timeout_list = self.timer_callbacks.disable_irq().lock();
            if timeout_list.is_empty() {
                return;
            }

            let mut callbacks = Vec::new();
            let current_time = self.read_time();
            while let Some(timer_callback) = timeout_list.peek() {
                if timer_callback.is_cancelled() {
                    timeout_list.pop();
                } else if timer_callback.expired_time <= current_time {
                    callbacks.push(timeout_list.pop().unwrap());
                } else {
                    break;
                }
            }
            callbacks
        };

        for callback in callbacks {
            callback.call();
        }
    }

    /// Creates a managed timer.
    pub fn create_timer<F>(self: &Arc<Self>, callback: F) -> Arc<Timer>
    where
        F: Fn(TimerGuard) + Send + Sync + 'static,
    {
        Timer::new(callback, self.clone())
    }
}

struct TimerCallback {
    expired_time: Duration,
    timer: Weak<Timer>,
    is_cancelled: AtomicBool,
}

impl TimerCallback {
    fn new(expired_time: Duration, timer: Weak<Timer>) -> Self {
        Self {
            expired_time,
            timer,
            is_cancelled: AtomicBool::new(false),
        }
    }

    fn cancel(&self) {
        self.is_cancelled.store(true, Ordering::Release);
    }

    fn is_cancelled(&self) -> bool {
        self.is_cancelled.load(Ordering::Acquire)
    }

    fn call(&self) {
        let Some(timer) = self.timer.upgrade() else {
            return;
        };

        let mut timer_guard = timer.lock();

        if self.is_cancelled() {
            return;
        }

        let interval = timer_guard.interval();
        if interval != Duration::ZERO {
            timer_guard.set_timeout(Timeout::After(interval));
        }

        (timer.registered_callback)(timer_guard);
    }
}

impl PartialEq for TimerCallback {
    fn eq(&self, other: &Self) -> bool {
        self.expired_time == other.expired_time
    }
}

impl Eq for TimerCallback {}

impl PartialOrd for TimerCallback {
    fn partial_cmp(&self, other: &Self) -> Option<CmpOrdering> {
        Some(self.cmp(other))
    }
}

impl Ord for TimerCallback {
    fn cmp(&self, other: &Self) -> CmpOrdering {
        self.expired_time.cmp(&other.expired_time).reverse()
    }
}
