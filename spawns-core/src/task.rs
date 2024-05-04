use crate::spawn_with_name;
use std::any::Any;
use std::borrow::Cow;
use std::cell::Cell;
use std::fmt::{self, Debug, Display, Formatter};
use std::future::Future;
use std::mem;
use std::mem::ManuallyDrop;
use std::panic::{self, AssertUnwindSafe};
use std::pin::Pin;
use std::ptr;
use std::sync::atomic::{AtomicPtr, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

thread_local! {
    static RUNNING_ID: Cell<RawId> = const { Cell::new(RawId(0)) };
}

#[repr(usize)]
#[derive(Clone, Copy, PartialEq)]
enum Stage {
    Running = 0,
    Finished = 1,
    Detached = 2,
}

impl From<usize> for Stage {
    fn from(v: usize) -> Self {
        match v {
            0 => Stage::Running,
            1 => Stage::Finished,
            2 => Stage::Detached,
            _ => unreachable!("{v} is not valid for Stage"),
        }
    }
}

enum JoinResult<T> {
    Empty,
    Joining(Waker),
    Joined,
    Finished(Result<T, InnerJoinError>),
    Detached,
}

#[derive(Clone, Copy)]
#[cfg_attr(test, derive(Debug, PartialEq))]
struct RawId(u64);

impl From<RawId> for Id {
    fn from(id: RawId) -> Id {
        Id(id.0)
    }
}

impl RawId {
    fn run(self) -> RawId {
        RUNNING_ID.with(|id| {
            let previous = id.get();
            id.set(self);
            previous
        })
    }

    pub(crate) fn enter(&self) -> IdScope {
        let id = RawId(self.0);
        let previous = id.run();
        IdScope { previous }
    }

    fn next() -> RawId {
        // We could reclaim id with help from `Drop` of `JoinHandle` and `Task`.
        static ID: AtomicU64 = AtomicU64::new(1);
        Self(ID.fetch_add(1, Ordering::Relaxed))
    }
}

/// Unique among running tasks.
///
/// There is no guarantee about its uniqueness among all spawned tasks. One could be reclaimed
/// after task finished and joined.
///
/// # Cautions
/// * Runtime should not rely on its uniqueness after task dropped.
/// * Client should not rely on tis uniqueness after task joined.
///
/// It is intentional to not `Copy` but `Clone` to make it verbose to create a new one to avoid abusing.
#[derive(Clone, Hash, PartialEq, Eq)]
pub struct Id(u64);

struct IdScope {
    previous: RawId,
}

impl Drop for IdScope {
    fn drop(&mut self) {
        self.previous.run();
    }
}

impl Id {
    fn as_raw(&self) -> RawId {
        RawId(self.0)
    }
}

impl Display for Id {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        if self.0 <= u32::MAX.into() {
            write!(f, "{:#010x}", self.0 as u32)
        } else {
            write!(f, "{:#018x}", self.0)
        }
    }
}

impl Debug for Id {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "TaskId({})", self)
    }
}

/// Thin wrapper around task to accommodate possible new members.
#[non_exhaustive]
pub struct Task {
    pub id: Id,
    pub name: Name,
    pub future: Box<dyn Future<Output = ()> + Send + 'static>,
}

impl Task {
    pub(crate) fn new<T: Send, F: Future<Output = T> + Send + 'static>(
        name: Name,
        future: F,
    ) -> (Self, JoinHandle<F::Output>) {
        let (id, future) = IdFuture::new(future);
        let future = TaskFuture::new(future);
        let handle = JoinHandle::new(
            id.as_raw(),
            future.joint.clone(),
            future.cancellation.clone(),
        );
        let task = Self {
            id,
            name,
            future: Box::new(future),
        };
        (task, handle)
    }

    pub fn name(&self) -> &str {
        self.name.as_str()
    }

    pub fn try_name(&self) -> Option<&str> {
        self.name.try_as_str()
    }
}

/// Task name.
#[derive(Clone, Default, Hash, PartialEq, Eq, Debug)]
pub struct Name {
    name: Option<Cow<'static, str>>,
}

impl Name {
    /// Returns task name as str. `unnamed` for unnamed task.
    pub fn as_str(&self) -> &str {
        self.try_as_str().unwrap_or("unnamed")
    }

    /// Returns task name as str. [None] for unnamed task.
    pub fn try_as_str(&self) -> Option<&str> {
        self.name.as_ref().map(|n| n.as_ref())
    }

    /// Decomposes this into [Cow] str.
    pub fn into(self) -> Option<Cow<'static, str>> {
        self.name
    }

    fn name(self, name: impl Into<Cow<'static, str>>) -> Self {
        Self {
            name: Some(name.into()),
        }
    }
}

/// Builder to spawn task with more options.
#[derive(Default)]
pub struct Builder {
    name: Name,
}

impl Builder {
    /// Constructs a builder to spawn task with more options.
    pub fn new() -> Self {
        Self::default()
    }

    /// Names spawning task.
    pub fn name(self, name: impl Into<Cow<'static, str>>) -> Self {
        Self {
            name: self.name.name(name),
        }
    }

    /// Spawns a task with configured options.
    pub fn spawn<T, F>(self, f: F) -> JoinHandle<T>
    where
        F: Future<Output = T> + Send + 'static,
        T: Send + 'static,
    {
        spawn_with_name(self.name, f)
    }
}

struct Joint<T> {
    stage: AtomicUsize,
    result: Mutex<JoinResult<T>>,
}

impl<T> Joint<T> {
    fn new() -> Self {
        Self {
            stage: AtomicUsize::new(Stage::Running as usize),
            result: Mutex::new(JoinResult::Empty),
        }
    }

    fn wake(&self, value: Result<T, InnerJoinError>) {
        let stage = self.stage();
        if stage != Stage::Running {
            return;
        }
        let mut lock = self.result.lock().unwrap();
        let result = mem::replace(&mut *lock, JoinResult::Finished(value));
        drop(lock);
        // Update stage after lock released as atomic Relaxed carrying no happen-before relationship.
        self.stage
            .store(Stage::Finished as usize, Ordering::Relaxed);
        match result {
            JoinResult::Empty => {}
            JoinResult::Joining(waker) => waker.wake(),
            JoinResult::Finished(_) | JoinResult::Joined => unreachable!("task completed already"),
            JoinResult::Detached => *self.result.lock().unwrap() = JoinResult::Detached,
        }
    }

    fn stage(&self) -> Stage {
        Stage::from(self.stage.load(Ordering::Relaxed))
    }
}

struct IdFuture<F: Future> {
    id: RawId,
    future: F,
}

impl<F: Future> IdFuture<F> {
    pub fn new(future: F) -> (Id, Self) {
        let id = RawId::next();
        let future = Self { id, future };
        (id.into(), future)
    }
}

impl<F: Future> Future for IdFuture<F> {
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let _scope = self.id.enter();
        let task = unsafe { self.get_unchecked_mut() };
        let future = unsafe { Pin::new_unchecked(&mut task.future) };
        future.poll(cx)
    }
}

struct TaskFuture<F: Future> {
    ready: bool,
    waker: Option<Box<Waker>>,
    cancellation: Arc<Cancellation>,
    joint: Arc<Joint<F::Output>>,
    future: F,
}

impl<F: Future> TaskFuture<F> {
    fn new(future: F) -> Self {
        Self {
            ready: false,
            waker: None,
            joint: Arc::new(Joint::new()),
            cancellation: Arc::new(Default::default()),
            future,
        }
    }
}
impl<F: Future> Drop for TaskFuture<F> {
    fn drop(&mut self) {
        if !self.ready {
            self.joint.wake(Err(InnerJoinError::Cancelled));
        }
    }
}

impl<F: Future> Future for TaskFuture<F> {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let task = unsafe { self.get_unchecked_mut() };
        if task.ready {
            return Poll::Ready(());
        } else if task.cancellation.is_cancelled() {
            task.joint.wake(Err(InnerJoinError::Cancelled));
            task.ready = true;
            return Poll::Ready(());
        }
        let future = unsafe { Pin::new_unchecked(&mut task.future) };
        match panic::catch_unwind(AssertUnwindSafe(|| future.poll(cx))) {
            Ok(Poll::Pending) => {
                let waker = match task.waker.take() {
                    None => Box::new(cx.waker().clone()),
                    Some(mut waker) => {
                        waker.as_mut().clone_from(cx.waker());
                        waker
                    }
                };
                let Ok(waker) = task.cancellation.update(waker) else {
                    task.joint.wake(Err(InnerJoinError::Cancelled));
                    task.ready = true;
                    return Poll::Ready(());
                };
                task.waker = waker;
                Poll::Pending
            }
            Ok(Poll::Ready(value)) => {
                task.joint.wake(Ok(value));
                task.ready = true;
                Poll::Ready(())
            }
            Err(err) => {
                task.joint.wake(Err(InnerJoinError::Panic(err)));
                task.ready = true;
                Poll::Ready(())
            }
        }
    }
}

#[derive(Default)]
struct Cancellation {
    waker: AtomicPtr<Waker>,
}

impl Cancellation {
    const CANCELLED: *mut Waker = Self::cancelled();

    const fn cancelled() -> *mut Waker {
        Cancellation::cancel as *const fn() as *mut Waker
    }

    pub fn is_cancelled(&self) -> bool {
        let current = self.waker.load(Ordering::Relaxed);
        ptr::eq(current, Self::CANCELLED)
    }

    pub fn cancel(&self) {
        let mut current = self.waker.load(Ordering::Relaxed);
        loop {
            if ptr::eq(current, Self::CANCELLED) {
                return;
            }
            match self.waker.compare_exchange(
                current,
                Self::CANCELLED,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Err(newer) => current = newer,
                _ => {
                    if !ptr::eq(current, ptr::null()) {
                        let waker = unsafe { Box::from_raw(current) };
                        waker.wake();
                    }
                    return;
                }
            }
        }
    }

    pub fn update(&self, mut waker: Box<Waker>) -> Result<Option<Box<Waker>>, ()> {
        let raw_waker = waker.as_mut() as *mut Waker;
        let mut current = self.waker.load(Ordering::Relaxed);
        loop {
            if ptr::eq(current, Self::CANCELLED) {
                return Err(());
            }
            match self.waker.compare_exchange(
                current,
                raw_waker,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Err(newer) => current = newer,
                _ => {
                    mem::forget(waker);
                    if !ptr::eq(current, ptr::null()) {
                        let waker = unsafe { Box::from_raw(current) };
                        return Ok(Some(waker));
                    }
                    return Ok(None);
                }
            }
        }
    }
}

impl Drop for Cancellation {
    fn drop(&mut self) {
        let waker = self.waker.load(Ordering::Relaxed);
        if ptr::eq(waker, ptr::null()) || ptr::eq(waker, Self::CANCELLED) {
            return;
        }
        let _ = unsafe { Box::from_raw(waker) };
    }
}

enum InnerJoinError {
    Cancelled,
    Panic(Box<dyn Any + Send + 'static>),
}

/// Error in polling [JoinHandle].
pub struct JoinError {
    id: Id,
    inner: InnerJoinError,
}

impl JoinError {
    /// Consumes this into panic if it represents a panic error.
    pub fn try_into_panic(self) -> Result<Box<dyn Any + Send + 'static>, JoinError> {
        match self.inner {
            InnerJoinError::Panic(p) => Ok(p),
            _ => Err(self),
        }
    }

    /// Consumes this into panic if it represents a panic error, panic otherwise.
    pub fn into_panic(self) -> Box<dyn Any + Send + 'static> {
        match self.try_into_panic() {
            Ok(panic) => panic,
            Err(err) => panic!("task {} does not panic, but cancelled", err.id),
        }
    }

    /// Resumes the catched panic if it represents a panic error, otherwise panic with error
    /// reason.
    pub fn resume_panic(self) -> ! {
        panic::resume_unwind(self.into_panic())
    }

    /// Returns true if task panic in polling.
    pub fn is_panic(&self) -> bool {
        matches!(&self.inner, InnerJoinError::Panic(_))
    }

    /// Returns true if task is cancelled.
    pub fn is_cancelled(&self) -> bool {
        matches!(&self.inner, InnerJoinError::Cancelled)
    }
}

impl Debug for JoinError {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.inner {
            InnerJoinError::Cancelled => write!(fmt, "JoinError::Cancelled({:?})", self.id),
            InnerJoinError::Panic(_panic) => write!(fmt, "JoinError::Panic({:?}, ...)", self.id),
        }
    }
}

impl Display for JoinError {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.inner {
            InnerJoinError::Cancelled => write!(fmt, "task {} was cancelled", self.id),
            InnerJoinError::Panic(_panic) => write!(fmt, "task {} panicked", self.id),
        }
    }
}

impl std::error::Error for JoinError {}

/// Handle to cancel associated task.
#[derive(Clone)]
pub struct CancelHandle {
    cancellation: Arc<Cancellation>,
}

unsafe impl Send for CancelHandle {}
unsafe impl Sync for CancelHandle {}

impl CancelHandle {
    fn new(cancellation: Arc<Cancellation>) -> Self {
        Self { cancellation }
    }

    /// Cancels associated task with this handle.
    ///
    /// Cancellation is inherently concurrent with task execution. Currently, there is no guarantee
    /// about promptness, the task could even run to complete normally after cancellation.
    pub fn cancel(&self) {
        self.cancellation.cancel()
    }
}

/// An owned permission to cancel task on [Drop] besides join and cancel on demand.
#[must_use = "task will be cancelled on drop, detach it if this is undesired"]
pub struct TaskHandle<T> {
    handle: JoinHandle<T>,
}

impl<T> TaskHandle<T> {
    /// Gets id of associated task.
    ///
    /// # Cautions
    /// See cautions of [Id].
    pub fn id(&self) -> Id {
        self.handle.id()
    }

    /// Detaches task from permission to cancel on [Drop].
    pub fn detach(self) -> JoinHandle<T> {
        let task = ManuallyDrop::new(self);
        unsafe { ptr::read(&task.handle) }
    }

    /// Cancels owning task.
    ///
    /// Cancellation is inherently concurrent with task execution. Currently, there is no guarantee
    /// about promptness, the task could even run to complete normally after cancellation.
    pub fn cancel(self) -> JoinHandle<T> {
        self.handle.cancel();
        self.detach()
    }

    /// Creates a handle to cancel associated task.
    pub fn cancel_handle(&self) -> CancelHandle {
        self.handle.cancel_handle()
    }
}

impl<T> Drop for TaskHandle<T> {
    fn drop(&mut self) {
        self.handle.cancel()
    }
}

impl<T> Future for TaskHandle<T> {
    type Output = Result<T, JoinError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let handle = unsafe { Pin::new_unchecked(&mut self.get_unchecked_mut().handle) };
        handle.poll(cx)
    }
}

/// Handle to join or cancel associated task.
pub struct JoinHandle<T> {
    id: RawId,
    joint: Arc<Joint<T>>,
    cancellation: Arc<Cancellation>,
}

impl<T> JoinHandle<T> {
    /// Gets id of associated task.
    ///
    /// # Cautions
    /// See cautions of [Id].
    pub fn id(&self) -> Id {
        self.id.into()
    }

    fn new(id: RawId, joint: Arc<Joint<T>>, cancellation: Arc<Cancellation>) -> Self {
        Self {
            id,
            joint,
            cancellation,
        }
    }

    /// Cancels associated task with this handle.
    ///
    /// Cancellation is inherently concurrent with task execution. Currently, there is no guarantee
    /// about promptness, the task could even run to complete normally after cancellation.
    pub fn cancel(&self) {
        self.cancellation.cancel()
    }

    /// Attaches to associated task to gain cancel on [Drop] permission.
    pub fn attach(self) -> TaskHandle<T> {
        TaskHandle { handle: self }
    }

    #[cfg(test)]
    unsafe fn clone(&self) -> Self {
        Self {
            id: self.id,
            joint: self.joint.clone(),
            cancellation: self.cancellation.clone(),
        }
    }

    /// Creates a handle to cancel associated task.
    pub fn cancel_handle(&self) -> CancelHandle {
        CancelHandle::new(self.cancellation.clone())
    }
}

impl<T> Drop for JoinHandle<T> {
    fn drop(&mut self) {
        self.joint
            .stage
            .store(Stage::Detached as usize, Ordering::Relaxed);
        *self.joint.result.lock().unwrap() = JoinResult::Detached;
    }
}

impl<T> Future for JoinHandle<T> {
    type Output = Result<T, JoinError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let stage = self.joint.stage();
        if stage == Stage::Finished {
            let mut lock = self.joint.result.lock().unwrap();
            let result = mem::replace(&mut *lock, JoinResult::Joined);
            drop(lock);
            return match result {
                JoinResult::Finished(result) => Poll::Ready(result.map_err(|inner| JoinError {
                    id: self.id.into(),
                    inner,
                })),
                JoinResult::Joined => panic!("task({}) already joined", Id(self.id.0)),
                JoinResult::Detached => panic!("task({}) already detached", Id(self.id.0)),
                _ => unreachable!("get no task result after stage finished"),
            };
        }
        let waker = cx.waker().clone();
        let mut lock = self.joint.result.lock().unwrap();
        let result = mem::replace(&mut *lock, JoinResult::Joining(waker));
        drop(lock);
        match result {
            JoinResult::Finished(result) => {
                *self.joint.result.lock().unwrap() = JoinResult::Joined;
                Poll::Ready(result.map_err(|inner| JoinError {
                    id: self.id.into(),
                    inner,
                }))
            }
            JoinResult::Empty | JoinResult::Joining(_) => Poll::Pending,
            JoinResult::Joined => panic!("task({}) already joined", Id(self.id.0)),
            JoinResult::Detached => panic!("task({}) already detached", Id(self.id.0)),
        }
    }
}

/// Gets running task's id. Panic if no reside in managed async task.
///
/// # Cautions
/// See cautions of [Id].
pub fn id() -> Id {
    try_id().expect("id(): no task running")
}

/// Gets running task's id. [None] if no reside in managed async task.
///
/// # Cautions
/// See cautions of [Id].
pub fn try_id() -> Option<Id> {
    let id = RUNNING_ID.get();
    if id.0 == 0 {
        None
    } else {
        Some(id.into())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;
    use std::time::{Duration, Instant};

    use super::*;
    use futures::executor::block_on;
    use futures::task::noop_waker;
    use std::future::{pending, ready};

    use static_assertions::*;

    assert_impl_all!(Id: Clone);
    assert_not_impl_any!(Id: Copy);

    assert_impl_all!(JoinHandle<()>: Send, Sync);
    assert_not_impl_any!(JoinHandle<()>: Clone, Copy);

    assert_impl_all!(TaskHandle<()>: Send, Sync);
    assert_not_impl_any!(TaskHandle<()>: Clone, Copy);

    assert_impl_all!(CancelHandle: Send, Sync);

    #[test]
    #[should_panic(expected = "no task running")]
    fn id_no() {
        id();
    }

    #[test]
    fn id_ok() {
        let (id, future) = IdFuture::new(async { id() });
        assert_eq!(block_on(future), id);
    }

    #[test]
    fn try_id_no() {
        assert_eq!(try_id(), None);
    }

    #[test]
    fn try_id_ok() {
        let (id, future) = IdFuture::new(async { try_id() });
        assert_eq!(block_on(future), Some(id));
    }

    #[test]
    fn id_display() {
        assert_eq!(Id(2).to_string(), "0x00000002");
        assert_eq!(Id(u32::MAX as u64).to_string(), "0xffffffff");
        assert_eq!(Id(u32::MAX as u64 + 1).to_string(), "0x0000000100000000");
    }

    #[test]
    fn id_debug() {
        assert_eq!(format!("{:?}", Id(2)), "TaskId(0x00000002)");
        assert_eq!(format!("{:?}", Id(u32::MAX as u64)), "TaskId(0xffffffff)");
        assert_eq!(
            format!("{:?}", Id(u32::MAX as u64 + 1)),
            "TaskId(0x0000000100000000)"
        );
    }

    #[test]
    fn id_raw() {
        let raw_id = RawId(2);
        let id = Id::from(raw_id);
        assert_eq!(id.as_raw(), raw_id);
    }

    #[test]
    fn name_unnamed() {
        let name = Name::default();
        assert_eq!(name.try_as_str(), None);
        assert_eq!(name.as_str(), "unnamed");
        assert_eq!(name.into(), None);
    }

    #[test]
    fn name_named() {
        let name = Name::default().name("named");
        assert_eq!(name.try_as_str(), Some("named"));
        assert_eq!(name.as_str(), "named");
        assert_eq!(name.into(), Some(Cow::Borrowed("named")));
    }

    #[test]
    fn task_unnamed() {
        let (task, _handle) = Task::new(Name::default(), pending::<()>());
        assert_eq!(task.try_name(), None);
        assert_eq!(task.name(), "unnamed");
    }

    #[test]
    fn task_named() {
        let (task, _handle) = Task::new(Name::default().name("named"), pending::<()>());
        assert_eq!(task.try_name(), Some("named"));
        assert_eq!(task.name(), "named");
    }

    #[test]
    fn task_id() {
        let (task, handle) = Task::new(Name::default(), async { id() });
        block_on(Box::into_pin(task.future));
        assert_eq!(block_on(handle).unwrap(), task.id);
    }

    #[test]
    fn task_try_id() {
        let (task, handle) = Task::new(Name::default(), async { try_id() });
        block_on(Box::into_pin(task.future));
        assert_eq!(block_on(handle).unwrap(), Some(task.id));
    }

    #[test]
    fn task_cancel() {
        let (task, handle) = Task::new(Name::default().name("named"), pending::<()>());
        handle.cancel();
        block_on(Box::into_pin(task.future));
        let err = block_on(handle).unwrap_err();
        assert_eq!(err.is_cancelled(), true);
    }

    #[test]
    fn task_cancel_handle() {
        let (task, handle) = Task::new(Name::default().name("named"), pending::<()>());
        handle.cancel_handle().cancel();
        block_on(Box::into_pin(task.future));
        let err = block_on(handle).unwrap_err();
        assert_eq!(err.is_cancelled(), true);
    }

    #[test]
    fn task_cancel_passively() {
        let (task, handle) = Task::new(Name::default(), ready(()));
        drop(task);
        let err = block_on(handle).unwrap_err();
        assert_eq!(err.is_cancelled(), true);
    }

    #[test]
    fn task_cancel_after_polling() {
        let (task, handle) = Task::new(Name::default(), pending::<()>());
        let mut task = Box::into_pin(task.future);
        let noop_waker = noop_waker();
        let mut cx = Context::from_waker(&noop_waker);
        assert_eq!(task.as_mut().poll(&mut cx), Poll::Pending);
        handle.cancel();
        assert_eq!(task.as_mut().poll(&mut cx), Poll::Ready(()));
        let err = block_on(handle).unwrap_err();
        assert_eq!(err.is_cancelled(), true);
    }

    #[test]
    fn task_cancel_after_completed() {
        let (task, handle) = Task::new(Name::default(), ready(()));
        block_on(Box::into_pin(task.future));
        handle.cancel();
        block_on(handle).unwrap();
    }

    #[test]
    #[should_panic(expected = "panic in task")]
    fn task_cancel_after_paniced() {
        let (task, handle) = Task::new(Name::default(), async { panic!("panic in task") });
        block_on(Box::into_pin(task.future));
        handle.cancel();
        block_on(handle).unwrap_err().resume_panic();
    }

    #[test]
    #[should_panic(expected = "panic in task")]
    fn join_handle_join_paniced() {
        let (task, handle) = Task::new(Name::default(), async { panic!("panic in task") });
        block_on(Box::into_pin(task.future));
        block_on(handle).unwrap_err().resume_panic();
    }

    #[test]
    fn join_handle_join_running() {
        let (task, handle) = Task::new(Name::default(), ready(()));
        std::thread::spawn(move || {
            block_on(Box::into_pin(task.future));
        });
        block_on(handle).unwrap();
    }

    #[test]
    fn join_handle_join_finished() {
        let (task, handle) = Task::new(Name::default(), ready(()));
        block_on(Box::into_pin(task.future));
        block_on(handle).unwrap();
    }

    #[test]
    fn join_handle_join_finished_concurrently() {
        let (task, handle) = Task::new(Name::default(), ready(()));
        block_on(Box::into_pin(task.future));
        handle
            .joint
            .stage
            .store(Stage::Running as usize, Ordering::Relaxed);
        block_on(handle).unwrap();
    }

    #[test]
    #[should_panic(expected = "already joined")]
    fn join_handle_join_joined() {
        let (task, mut handle) = Task::new(Name::default(), ready(()));
        block_on(Box::into_pin(task.future));
        let noop_waker = noop_waker();
        let mut cx = Context::from_waker(&noop_waker);
        let pinned = unsafe { Pin::new_unchecked(&mut handle) };
        assert!(pinned.poll(&mut cx).is_ready());
        let pinned = unsafe { Pin::new_unchecked(&mut handle) };
        let _ = pinned.poll(&mut cx);
    }

    #[test]
    #[should_panic(expected = "already detached")]
    fn join_handle_join_detached() {
        let (_task, handle) = Task::new(Name::default(), ready(()));
        unsafe {
            drop(handle.clone());
        }
        let _ = block_on(handle);
    }

    #[test]
    fn join_handle_detached() {
        let (task, handle) = Task::new(Name::default(), ready(()));
        drop(handle);
        block_on(Box::into_pin(task.future));
    }

    #[test]
    fn join_handle_detached_concurrently() {
        let (task, handle) = Task::new(Name::default(), ready(()));
        *handle.joint.result.lock().unwrap() = JoinResult::Detached;
        block_on(Box::into_pin(task.future));
    }

    #[test]
    fn task_handle_join() {
        let (task, handle) = Task::new(Name::default(), async { id() });
        let handle = handle.attach();
        block_on(Box::into_pin(task.future));
        let id = handle.id();
        assert_eq!(block_on(handle).unwrap(), id);
    }

    #[test]
    fn task_handle_cancel() {
        let (task, handle) = Task::new(Name::default(), pending::<()>());
        std::thread::spawn(move || {
            block_on(Box::into_pin(task.future));
        });
        let handle = handle.attach();
        let err = block_on(async move { handle.cancel().await.unwrap_err() });
        assert_eq!(err.is_cancelled(), true);
    }

    struct Sleep {
        deadline: Instant,
    }

    impl Sleep {
        fn new(timeout: Duration) -> Sleep {
            Sleep {
                deadline: Instant::now() + timeout,
            }
        }
    }

    impl Future for Sleep {
        type Output = ();

        fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            let timeout = self.deadline.saturating_duration_since(Instant::now());
            if timeout.is_zero() {
                return Poll::Ready(());
            }
            let waker = cx.waker().clone();
            std::thread::spawn(move || {
                std::thread::sleep(timeout);
                waker.wake();
            });
            Poll::Pending
        }
    }

    #[test]
    fn task_handle_cancel_on_drop() {
        let (task, handle) = Task::new(Name::default(), pending::<()>());
        let handle = handle.attach();
        drop(handle);
        block_on(Box::into_pin(task.future));
    }

    #[test]
    fn task_handle_detach() {
        let cancelled = Arc::new(AtomicBool::new(true));
        let (task, handle) = Task::new(Name::default(), {
            let cancelled = cancelled.clone();
            async move {
                Sleep::new(Duration::from_millis(500)).await;
                cancelled.store(false, Ordering::Relaxed);
            }
        });
        let handle = handle.attach().detach();
        drop(handle);
        block_on(Box::into_pin(task.future));
        assert_eq!(cancelled.load(Ordering::Relaxed), false);
    }
}
