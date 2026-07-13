// SPDX-License-Identifier: Apache-2.0

use crate::hooks::{task_observer_hook, TaskPollOutcome};
use std::future::Future;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

const MAX_MODEL_TASKS: usize = 8;

#[derive(Default)]
struct CompletionState {
    finished: bool,
    waker: Option<Waker>,
}

#[derive(Default)]
struct TaskCompletion {
    state: Mutex<CompletionState>,
}

impl TaskCompletion {
    fn complete(&self) {
        let waker = {
            let mut state = self.state.lock().expect("task completion lock poisoned");
            state.finished = true;
            state.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }

    fn poll(&self, cx: &Context<'_>) -> Poll<()> {
        let mut state = self.state.lock().expect("task completion lock poisoned");
        if state.finished {
            return Poll::Ready(());
        }

        let replace_waker = match state.waker.as_ref() {
            Some(waker) => !waker.will_wake(cx.waker()),
            None => true,
        };
        if replace_waker {
            state.waker = Some(cx.waker().clone());
        }
        Poll::Pending
    }
}

/// A handle that resolves when its associated `TaskSet` task reaches a terminal
/// state, including a contained panic.
pub struct TaskHandle {
    completion: Arc<TaskCompletion>,
}

impl Future for TaskHandle {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.get_mut().completion.poll(cx)
    }
}

impl Unpin for TaskHandle {}

/// A collection of non-Send futures registered before native execution.
pub struct TaskSet {
    tasks: Vec<Pin<Box<dyn Future<Output = ()> + 'static>>>,
}

impl TaskSet {
    /// Creates an empty task collection.
    #[must_use]
    pub const fn new() -> Self {
        Self { tasks: Vec::new() }
    }

    /// Registers one task and returns a handle that resolves on completion.
    ///
    /// The task limit mirrors the private engine's eight model-task limit.
    ///
    /// # Panics
    ///
    /// Panics when more than eight tasks are registered.
    pub fn spawn<F>(&mut self, future: F) -> TaskHandle
    where
        F: Future<Output = ()> + 'static,
    {
        assert!(
            self.tasks.len() < MAX_MODEL_TASKS,
            "laplace: engine model-task limit is 8"
        );

        let task = self.tasks.len() as u64;
        let completion = Arc::new(TaskCompletion::default());
        if let Some(hook) = task_observer_hook() {
            hook.task_registered(task);
        }

        self.tasks.push(Box::pin(ObservedTask {
            task,
            future: Box::pin(future) as Pin<Box<dyn Future<Output = ()> + 'static>>,
            completion: Some(Arc::clone(&completion)),
            attempt: 0,
            finished: false,
        }));

        TaskHandle { completion }
    }

    /// Consumes the collection for the native runner.
    #[doc(hidden)]
    #[must_use]
    pub fn into_tasks(self) -> Vec<Pin<Box<dyn Future<Output = ()> + 'static>>> {
        self.tasks
    }
}

impl Default for TaskSet {
    fn default() -> Self {
        Self::new()
    }
}

pub(crate) struct ObservedTask<F: ?Sized = dyn Future<Output = ()> + 'static> {
    task: u64,
    future: Pin<Box<F>>,
    completion: Option<Arc<TaskCompletion>>,
    attempt: u64,
    finished: bool,
}

impl<F: ?Sized> Unpin for ObservedTask<F> {}

impl<F> Future for ObservedTask<F>
where
    F: Future<Output = ()> + ?Sized,
{
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        if this.finished {
            return Poll::Ready(());
        }

        let attempt = this.attempt;
        this.attempt = this.attempt.saturating_add(1);
        let hook = task_observer_hook();
        if let Some(hook) = hook.as_ref() {
            hook.poll_started(this.task, attempt);
        }

        let outcome = match catch_unwind(AssertUnwindSafe(|| this.future.as_mut().poll(cx))) {
            Ok(Poll::Pending) => TaskPollOutcome::Pending,
            Ok(Poll::Ready(())) => TaskPollOutcome::Ready,
            Err(_) => TaskPollOutcome::Panicked,
        };

        if let Some(hook) = hook.as_ref() {
            hook.poll_completed(this.task, attempt, outcome);
        }

        match outcome {
            TaskPollOutcome::Pending => Poll::Pending,
            TaskPollOutcome::Ready | TaskPollOutcome::Panicked => {
                this.finished = true;
                if let Some(completion) = this.completion.as_ref() {
                    completion.complete();
                }
                if let Some(hook) = hook.as_ref() {
                    hook.task_completed(this.task);
                }
                Poll::Ready(())
            }
        }
    }
}

impl<F> ObservedTask<F>
where
    F: Future<Output = ()> + 'static,
{
    pub(crate) fn without_completion(task: u64, future: F) -> Self {
        Self {
            task,
            future: Box::pin(future),
            completion: None,
            attempt: 0,
            finished: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        clear_async_spawn_hook, clear_task_observer_hook, install_task_observer_hook,
        TaskObserverHook, TaskPollOutcome,
    };
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, MutexGuard, PoisonError,
    };
    use std::task::{Context, Poll, Waker};

    static TEST_GUARD: Mutex<()> = Mutex::new(());

    fn serial() -> MutexGuard<'static, ()> {
        TEST_GUARD.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn poll_once<F: Future + ?Sized>(future: Pin<&mut F>) -> Poll<F::Output> {
        let mut cx = Context::from_waker(Waker::noop());
        future.poll(&mut cx)
    }

    #[test]
    fn task_future_completes_without_observer_hook() {
        let _serial = serial();
        clear_task_observer_hook();

        let completed = Arc::new(AtomicBool::new(false));
        let marker = Arc::clone(&completed);
        let mut set = TaskSet::new();
        let handle = set.spawn(async move {
            marker.store(true, Ordering::SeqCst);
        });

        let mut tasks = set.into_tasks();
        let mut task = tasks.pop().expect("one task");
        assert!(matches!(poll_once(task.as_mut()), Poll::Ready(())));
        assert!(completed.load(Ordering::SeqCst));

        let mut handle = Box::pin(handle);
        assert!(matches!(poll_once(handle.as_mut()), Poll::Ready(())));
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    enum Event {
        Registered(u64),
        Dynamic(u64),
        Started(u64, u64),
        Completed(u64, u64, TaskPollOutcome),
        Finished(u64),
    }

    struct RecordingTaskHook {
        events: Mutex<Vec<Event>>,
    }

    impl RecordingTaskHook {
        fn new() -> Self {
            Self {
                events: Mutex::new(Vec::new()),
            }
        }

        fn events(&self) -> Vec<Event> {
            self.events.lock().expect("task hook events lock").clone()
        }
    }

    impl TaskObserverHook for RecordingTaskHook {
        fn task_registered(&self, task: u64) {
            self.events
                .lock()
                .expect("task hook events lock")
                .push(Event::Registered(task));
        }

        fn dynamic_task_spawned(&self, task: u64) {
            self.events
                .lock()
                .expect("task hook events lock")
                .push(Event::Dynamic(task));
        }

        fn poll_started(&self, task: u64, attempt: u64) {
            self.events
                .lock()
                .expect("task hook events lock")
                .push(Event::Started(task, attempt));
        }

        fn poll_completed(&self, task: u64, attempt: u64, outcome: TaskPollOutcome) {
            self.events
                .lock()
                .expect("task hook events lock")
                .push(Event::Completed(task, attempt, outcome));
        }

        fn task_completed(&self, task: u64) {
            self.events
                .lock()
                .expect("task hook events lock")
                .push(Event::Finished(task));
        }
    }

    #[test]
    fn observer_callbacks_follow_registration_poll_and_completion_order() {
        let _serial = serial();
        clear_task_observer_hook();
        let hook = Arc::new(RecordingTaskHook::new());
        install_task_observer_hook(hook.clone());

        let mut set = TaskSet::new();
        let _handle = set.spawn(async {});
        let mut tasks = set.into_tasks();
        let mut task = tasks.pop().expect("one task");
        assert!(matches!(poll_once(task.as_mut()), Poll::Ready(())));

        clear_task_observer_hook();
        assert_eq!(
            hook.events(),
            vec![
                Event::Registered(0),
                Event::Started(0, 0),
                Event::Completed(0, 0, TaskPollOutcome::Ready),
                Event::Finished(0),
            ]
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn native_spawn_reports_a_reserved_dynamic_task_id() {
        let _serial = serial();
        clear_async_spawn_hook();
        let hook = Arc::new(RecordingTaskHook::new());
        install_task_observer_hook(hook.clone());

        let handle = crate::spawn::spawn_task(async {});
        handle.await.expect("native task must complete");

        clear_task_observer_hook();
        let events = hook.events();
        let dynamic = events
            .iter()
            .find_map(|event| match event {
                Event::Dynamic(task) => Some(*task),
                _ => None,
            })
            .expect("native spawn must report its dynamic task");
        assert!(
            dynamic >= (1_u64 << 63),
            "dynamic task id must use the reserved namespace: {dynamic}"
        );
        assert!(events.contains(&Event::Started(dynamic, 0)));
        assert!(events.contains(&Event::Completed(dynamic, 0, TaskPollOutcome::Ready)));
        assert!(events.contains(&Event::Finished(dynamic)));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn native_spawns_receive_distinct_dynamic_task_ids() {
        let _serial = serial();
        clear_async_spawn_hook();
        let hook = Arc::new(RecordingTaskHook::new());
        install_task_observer_hook(hook.clone());

        let first = crate::spawn::spawn_task(async {});
        let second = crate::spawn::spawn_task(async {});
        first.await.expect("first native task must complete");
        second.await.expect("second native task must complete");

        clear_task_observer_hook();
        let dynamic = hook
            .events()
            .into_iter()
            .filter_map(|event| match event {
                Event::Dynamic(task) => Some(task),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(dynamic.len(), 2);
        assert_ne!(dynamic[0], dynamic[1]);
    }

    #[test]
    fn panicked_task_completes_its_handle() {
        let _serial = serial();
        clear_task_observer_hook();
        let mut set = TaskSet::new();
        let handle = set.spawn(async {
            panic!("task panic is contained by the task wrapper");
        });

        let mut tasks = set.into_tasks();
        let mut task = tasks.pop().expect("one task");
        assert!(matches!(poll_once(task.as_mut()), Poll::Ready(())));

        let mut handle = Box::pin(handle);
        assert!(matches!(poll_once(handle.as_mut()), Poll::Ready(())));
    }

    #[test]
    #[should_panic(expected = "engine model-task limit is 8")]
    fn ninth_task_is_rejected_at_runtime() {
        let _serial = serial();
        clear_task_observer_hook();
        let mut set = TaskSet::new();
        for _ in 0..9 {
            drop(set.spawn(async {}));
        }
    }
}
