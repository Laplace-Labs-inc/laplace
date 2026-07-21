// SPDX-License-Identifier: Apache-2.0
//! Model-thread spawn seam.
//!
//! [`spawn`] routes a unit-returning model thread through an installed
//! [`SpawnHook`](crate::hooks::SpawnHook); [`spawn_task`] similarly routes a
//! fire-and-forget async future through an [`AsyncSpawnHook`](crate::hooks::AsyncSpawnHook).
//! With no hook installed, the two seams use their native thread and tokio
//! implementations respectively.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::thread::JoinHandle;

use crate::hooks::{async_spawn_hook, next_native_dynamic_task_id, spawn_hook, task_observer_hook};
use crate::task_set::ObservedTask;

enum JoinMode {
    Std(JoinHandle<()>),
    Engine,
}

/// Join handle returned by [`spawn`].
///
/// Without an installed hook this wraps a real `std::thread::JoinHandle<()>`.
/// With an engine hook installed, join ownership stays with the engine runtime,
/// so [`JoinToken::join`] is a no-op success.
pub struct JoinToken {
    mode: JoinMode,
}

impl JoinToken {
    fn from_std(handle: JoinHandle<()>) -> Self {
        Self {
            mode: JoinMode::Std(handle),
        }
    }

    /// Creates an engine-owned join token.
    ///
    /// Engine hooks return this after handing the closure to their own runtime.
    #[must_use]
    pub const fn engine() -> Self {
        Self {
            mode: JoinMode::Engine,
        }
    }

    /// Waits for a free-tier thread or acknowledges an engine-owned thread.
    ///
    /// # Errors
    ///
    /// Returns the panic payload from the underlying std thread in free tier.
    pub fn join(self) -> std::thread::Result<()> {
        match self.mode {
            JoinMode::Std(handle) => handle.join(),
            JoinMode::Engine => Ok(()),
        }
    }
}

/// Spawns a unit-returning model thread.
///
/// If a hook is installed, the closure is routed to that hook. Otherwise it is
/// executed on a normal OS thread via `std::thread::spawn`.
#[must_use]
pub fn spawn<F>(f: F) -> JoinToken
where
    F: FnOnce() + Send + 'static,
{
    if let Some(hook) = spawn_hook() {
        return hook.spawn(Box::new(f));
    }

    JoinToken::from_std(std::thread::spawn(f))
}

/// Terminal state reported by an engine-owned task control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskControlState {
    /// The task future completed normally.
    Finished,
    /// The task was cancelled by [`TaskHandle::abort`].
    Cancelled,
    /// The task future panicked during model execution.
    Panicked,
}

/// Engine-side control surface for one [`TaskHandle`].
pub trait TaskControl: Send + Sync {
    /// Polls the modelled task's completion state.
    fn poll(&self, cx: &mut Context<'_>) -> Poll<TaskControlState>;

    /// Requests cancellation. The engine may defer applying the request until
    /// the current task poll has returned.
    fn abort(&self);

    /// Returns whether the task reached a terminal state.
    fn is_finished(&self) -> bool;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TaskJoinErrorKind {
    Cancelled,
    Panic,
}

/// Error returned when a [`TaskHandle`] does not complete normally.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskJoinError {
    kind: TaskJoinErrorKind,
}

impl TaskJoinError {
    fn cancelled() -> Self {
        Self {
            kind: TaskJoinErrorKind::Cancelled,
        }
    }

    fn panic() -> Self {
        Self {
            kind: TaskJoinErrorKind::Panic,
        }
    }

    /// Returns whether the task was cancelled before normal completion.
    #[must_use]
    pub const fn is_cancelled(self) -> bool {
        matches!(self.kind, TaskJoinErrorKind::Cancelled)
    }

    /// Returns whether the task panicked during execution.
    #[must_use]
    pub const fn is_panic(self) -> bool {
        matches!(self.kind, TaskJoinErrorKind::Panic)
    }
}

impl std::fmt::Display for TaskJoinError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.is_cancelled() {
            "task was cancelled"
        } else {
            "task panicked"
        })
    }
}

impl std::error::Error for TaskJoinError {}

enum TaskHandleMode<T> {
    Native(tokio::task::JoinHandle<T>),
    ObservedNative {
        task: u64,
        handle: tokio::task::JoinHandle<()>,
        output: Arc<Mutex<Option<T>>>,
        panicked: Arc<AtomicBool>,
    },
    Engine {
        control: Box<dyn TaskControl>,
        output: Arc<Mutex<Option<T>>>,
    },
}

/// Tokio-compatible shadow of an async task join handle.
pub struct TaskHandle<T> {
    mode: TaskHandleMode<T>,
    /// Join reporting state; only the observed-native mode reports, because it
    /// is the only mode whose task carries a capture identity.
    requested: bool,
    resolved: bool,
}

impl<T> TaskHandle<T> {
    fn native(handle: tokio::task::JoinHandle<T>) -> Self {
        Self {
            mode: TaskHandleMode::Native(handle),
            requested: false,
            resolved: false,
        }
    }

    fn observed_native(
        task: u64,
        handle: tokio::task::JoinHandle<()>,
        output: Arc<Mutex<Option<T>>>,
        panicked: Arc<AtomicBool>,
    ) -> Self {
        Self {
            mode: TaskHandleMode::ObservedNative {
                task,
                handle,
                output,
                panicked,
            },
            requested: false,
            resolved: false,
        }
    }

    fn engine(control: Box<dyn TaskControl>, output: Arc<Mutex<Option<T>>>) -> Self {
        Self {
            mode: TaskHandleMode::Engine { control, output },
            requested: false,
            resolved: false,
        }
    }

    /// Requests cancellation of the task.
    pub fn abort(&self) {
        match &self.mode {
            TaskHandleMode::Native(handle) => handle.abort(),
            TaskHandleMode::ObservedNative { handle, .. } => handle.abort(),
            TaskHandleMode::Engine { control, .. } => control.abort(),
        }
    }

    /// Returns whether the task has reached a terminal state.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        match &self.mode {
            TaskHandleMode::Native(handle) => handle.is_finished(),
            TaskHandleMode::ObservedNative { handle, .. } => handle.is_finished(),
            TaskHandleMode::Engine { control, .. } => control.is_finished(),
        }
    }
}

impl<T> Future for TaskHandle<T> {
    type Output = Result<T, TaskJoinError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        if let TaskHandleMode::ObservedNative { task, .. } = &this.mode {
            if !this.requested {
                this.requested = true;
                if let Some(hook) = task_observer_hook() {
                    hook.join_requested(*task);
                }
            }
        }
        let outcome = Self::poll_mode(&mut this.mode, cx);
        if outcome.is_ready() && !this.resolved {
            this.resolved = true;
            if let TaskHandleMode::ObservedNative { task, .. } = &this.mode {
                if let Some(hook) = task_observer_hook() {
                    hook.join_resolved(*task);
                }
            }
        }
        outcome
    }
}

impl<T> TaskHandle<T> {
    fn poll_mode(
        mode: &mut TaskHandleMode<T>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<T, TaskJoinError>> {
        match mode {
            TaskHandleMode::Native(handle) => Pin::new(handle).poll(cx).map(|result| {
                result.map_err(|error| {
                    if error.is_cancelled() {
                        TaskJoinError::cancelled()
                    } else {
                        TaskJoinError::panic()
                    }
                })
            }),
            TaskHandleMode::ObservedNative {
                handle,
                output,
                panicked,
                ..
            } => Pin::new(handle).poll(cx).map(|result| {
                result
                    .map_err(|error| {
                        if error.is_cancelled() {
                            TaskJoinError::cancelled()
                        } else {
                            TaskJoinError::panic()
                        }
                    })
                    .and_then(|()| {
                        if panicked.load(Ordering::Acquire) {
                            Err(TaskJoinError::panic())
                        } else {
                            Ok(output
                                .lock()
                                .expect("observed task output lock poisoned")
                                .take()
                                .expect("observed task must publish its output"))
                        }
                    })
            }),
            TaskHandleMode::Engine { control, output } => match control.poll(cx) {
                Poll::Pending => Poll::Pending,
                Poll::Ready(TaskControlState::Finished) => {
                    let value = output
                        .lock()
                        .expect("task output lock poisoned")
                        .take()
                        .expect("finished task must publish its output");
                    Poll::Ready(Ok(value))
                }
                Poll::Ready(TaskControlState::Cancelled) => {
                    Poll::Ready(Err(TaskJoinError::cancelled()))
                }
                Poll::Ready(TaskControlState::Panicked) => Poll::Ready(Err(TaskJoinError::panic())),
            },
        }
    }
}

struct NativeObservedFuture<T, F> {
    future: Pin<Box<F>>,
    output: Arc<Mutex<Option<T>>>,
    panicked: Arc<AtomicBool>,
}

impl<T, F> Future for NativeObservedFuture<T, F>
where
    T: Send + 'static,
    F: Future<Output = T> + Send + 'static,
{
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            this.future.as_mut().poll(cx)
        })) {
            Ok(Poll::Pending) => Poll::Pending,
            Ok(Poll::Ready(value)) => {
                *this
                    .output
                    .lock()
                    .expect("observed task output lock poisoned") = Some(value);
                Poll::Ready(())
            }
            Err(payload) => {
                this.panicked.store(true, Ordering::Release);
                std::panic::resume_unwind(payload);
            }
        }
    }
}

impl<T> Unpin for TaskHandle<T> {}

/// Spawns an async task and returns a Tokio-compatible join shadow.
///
/// If an async spawn hook is installed, the future is routed to that hook and
/// its output is retained in a private slot until the engine reports normal
/// completion. Otherwise it is delegated to `tokio::spawn`.
///
/// # Panics
///
/// The engine-backed wrapper panics if its private output slot is poisoned.
pub fn spawn_task<T, F>(future: F) -> TaskHandle<T>
where
    T: Send + 'static,
    F: Future<Output = T> + Send + 'static,
{
    if let Some(hook) = async_spawn_hook() {
        let output = Arc::new(Mutex::new(None));
        let output_slot = Arc::clone(&output);
        let control = hook.spawn_task(Box::pin(async move {
            let value = future.await;
            *output_slot.lock().expect("task output lock poisoned") = Some(value);
        }));
        return TaskHandle::engine(control, output);
    }

    if let Some(hook) = task_observer_hook() {
        let task = next_native_dynamic_task_id();
        hook.dynamic_task_spawned(task);
        let output = Arc::new(Mutex::new(None));
        let panicked = Arc::new(AtomicBool::new(false));
        let observed_future = NativeObservedFuture {
            future: Box::pin(future),
            output: Arc::clone(&output),
            panicked: Arc::clone(&panicked),
        };
        let observed = ObservedTask::without_completion(task, observed_future);
        return TaskHandle::observed_native(task, tokio::spawn(observed), output, panicked);
    }
    TaskHandle::native(tokio::spawn(future))
}
