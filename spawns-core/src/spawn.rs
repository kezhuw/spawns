#[cfg(feature = "compat")]
use crate::find_spawn;
use crate::{JoinHandle, Name, Task};
use std::cell::RefCell;
use std::future::Future;
use std::thread_local;

thread_local! {
    static SPAWNER: RefCell<Option<&'static dyn Spawn>> = RefCell::new(None);
}

/// Trait to spawn task.
pub trait Spawn {
    fn spawn(&self, task: Task);
}

/// Scope where tasks are [spawn]ed through given [Spawn].
pub struct SpawnScope<'a> {
    spawner: &'a dyn Spawn,
    previous: Option<&'static dyn Spawn>,
}

fn exchange(spawner: Option<&dyn Spawn>) -> Option<&'static dyn Spawn> {
    SPAWNER.with_borrow_mut(|previous| unsafe {
        std::mem::replace(previous, std::mem::transmute(spawner))
    })
}

/// Enters a scope where new tasks will be [spawn]ed through given [Spawn].
pub fn enter(spawner: &dyn Spawn) -> SpawnScope<'_> {
    let previous = exchange(Some(spawner));
    SpawnScope { previous, spawner }
}

impl Drop for SpawnScope<'_> {
    fn drop(&mut self) {
        let current = exchange(self.previous.take()).expect("no spawner");
        assert!(std::ptr::eq(self.spawner, current));
    }
}

pub(crate) fn spawn_with_name<T, F>(name: Name, f: F) -> JoinHandle<T>
where
    F: Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    SPAWNER
        .with_borrow(|spawner| match spawner {
            Some(spawner) => {
                let (task, handle) = Task::new(name, f);
                spawner.spawn(task);
                Some(handle)
            }
            #[cfg(not(feature = "compat"))]
            None => None,
            #[cfg(feature = "compat")]
            None => match find_spawn() {
                Some(spawn) => {
                    let (task, handle) = Task::new(name, f);
                    spawn(task);
                    Some(handle)
                }
                None => None,
            },
        })
        .expect("no spawner")
}

/// Spawns a new task.
///
/// # Panics
/// 1. Panic if no spawner.
/// 2. Panic if [Spawn::spawn] panic.
pub fn spawn<T, F>(f: F) -> JoinHandle<T>
where
    F: Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    spawn_with_name(Name::default(), f)
}

#[cfg(test)]
mod tests {
    use crate::{enter, id, spawn, Builder, Spawn, Task};
    use futures::executor::block_on;
    use std::future::{pending, ready};

    #[derive(Default, Clone, Copy)]
    struct DropSpawner {}

    impl Spawn for DropSpawner {
        fn spawn(&self, _task: Task) {}
    }

    #[derive(Default, Clone, Copy)]
    struct ThreadSpawner {}

    impl Spawn for ThreadSpawner {
        fn spawn(&self, task: Task) {
            std::thread::Builder::new()
                .name(task.name().to_string())
                .spawn(move || {
                    let spawner = ThreadSpawner::default();
                    let _scope = enter(&spawner);
                    block_on(Box::into_pin(task.future));
                })
                .unwrap();
        }
    }

    #[cfg(not(feature = "compat"))]
    #[test]
    #[should_panic(expected = "no spawner")]
    fn no_spawner() {
        spawn(ready(()));
    }

    #[test]
    fn drop_spawner() {
        let spawner = DropSpawner::default();
        let _scope = enter(&spawner);
        let handle = spawn(ready(()));
        let err = block_on(handle).unwrap_err();
        assert!(err.is_cancelled());
    }

    #[test]
    fn thread_spawner_named() {
        let spawner = ThreadSpawner::default();
        let _scope = enter(&spawner);
        let handle = Builder::new()
            .name("task1")
            .spawn(async { std::thread::current().name().unwrap().to_string() });
        let name = block_on(handle).unwrap();
        assert_eq!(name, "task1");
    }

    #[test]
    fn thread_spawner_unnamed() {
        let spawner = ThreadSpawner::default();
        let _scope = enter(&spawner);
        let handle = spawn(async { std::thread::current().name().unwrap().to_string() });
        let name = block_on(handle).unwrap();
        assert_eq!(name, "unnamed");
    }

    #[test]
    fn thread_spawner_cascading_ready() {
        let spawner = ThreadSpawner::default();
        let _scope = enter(&spawner);
        let handle = spawn(async move { spawn(async { id() }) });
        let handle = block_on(handle).unwrap();
        let id = handle.id();
        assert_eq!(block_on(handle).unwrap(), id);
    }

    #[test]
    fn thread_spawner_cascading_cancel() {
        let spawner = ThreadSpawner::default();
        let _scope = enter(&spawner);
        let handle = spawn(async move { spawn(pending::<()>()) });
        let handle = block_on(handle).unwrap();
        handle.cancel();
        let err = block_on(handle).unwrap_err();
        assert!(err.is_cancelled());
    }

    #[cfg(feature = "compat")]
    mod compat {
        use super::*;
        use crate::{Compat, COMPATS};
        use linkme::distributed_slice;
        use std::cell::Cell;
        thread_local! {
            static DROP_SPAWNER: Cell<Option<DropSpawner>> = Cell::new(None);
        }

        #[distributed_slice(COMPATS)]
        pub static DROP_LOCAL: Compat = Compat::Local(drop_local);

        fn drop_spawn(task: Task) {
            DROP_SPAWNER.get().expect("no drop spawner").spawn(task)
        }

        fn drop_local() -> Option<fn(Task)> {
            DROP_SPAWNER.get().map(|_| drop_spawn as fn(Task))
        }

        thread_local! {
            static THREAD_SPAWNER: Cell<Option<ThreadSpawner>> = Cell::new(None);
        }

        #[distributed_slice(COMPATS)]
        pub static THREAD_LOCAL: Compat = Compat::Local(thread_local);

        #[cfg(feature = "test-compat-global1")]
        #[distributed_slice(COMPATS)]
        #[allow(deprecated)]
        pub static THREAD_GLOBAL: Compat = Compat::Global(thread_global);

        #[cfg(feature = "test-compat-global2")]
        #[distributed_slice(COMPATS)]
        pub static DROP_GLOBAL: Compat = Compat::NamedGlobal {
            name: "drop",
            spawn: drop_global,
        };

        #[cfg(feature = "test-compat-global2")]
        fn drop_global(task: Task) {
            DropSpawner::default().spawn(task)
        }

        fn thread_spawn(task: Task) {
            THREAD_SPAWNER.get().expect("no thread spawner").spawn(task)
        }

        fn thread_local() -> Option<fn(Task)> {
            THREAD_SPAWNER.get().map(|_| thread_spawn as fn(Task))
        }

        #[cfg(feature = "test-compat-global1")]
        fn thread_global(task: Task) {
            ThreadSpawner::default().spawn(task)
        }

        #[test]
        #[cfg(not(any(feature = "test-compat-global1", feature = "test-compat-global2")))]
        #[should_panic(expected = "no spawner")]
        fn no_spawner() {
            spawn(ready(()));
        }

        #[test]
        fn drop_spawner_local() {
            DROP_SPAWNER.set(Some(DropSpawner::default()));
            let handle = spawn(ready(()));
            let err = block_on(handle).unwrap_err();
            assert!(err.is_cancelled());
        }

        #[test]
        fn thread_spawner_local() {
            THREAD_SPAWNER.set(Some(ThreadSpawner::default()));
            let handle = Builder::new()
                .name("task2")
                .spawn(async { std::thread::current().name().unwrap().to_string() });
            let name = block_on(handle).unwrap();
            assert_eq!(name, "task2");
        }

        #[cfg(all(feature = "test-compat-global1", not(feature = "test-compat-global2")))]
        #[test]
        fn thread_spawner_global() {
            let handle = Builder::new()
                .name("thread_spawner_global")
                .spawn(async { std::thread::current().name().unwrap().to_string() });
            let name = block_on(handle).unwrap();
            assert_eq!(name, "thread_spawner_global");
        }

        #[cfg(feature = "test-compat-global2")]
        #[cfg(not(feature = "test-named-global"))]
        #[cfg(feature = "panic-multiple-global-spawners")]
        #[test]
        #[should_panic(expected = "multiple global spawners")]
        fn multiple_globals() {
            spawn(ready(()));
        }

        #[cfg(feature = "test-compat-global2")]
        #[cfg(not(feature = "test-named-global"))]
        #[cfg(not(feature = "panic-multiple-global-spawners"))]
        #[test]
        fn multiple_globals() {
            // The one chosen is indeterminate.
            spawn(ready(()));
        }

        // Rust runs all tests in one process for given features, so it is crucial to keep features
        // set unique for this test as it setup environment variable SPAWNS_GLOBAL_SPAWNER.
        #[cfg(feature = "test-compat-global2")]
        #[cfg(feature = "test-named-global")]
        #[cfg(feature = "panic-multiple-global-spawners")]
        #[test]
        fn multiple_globals_choose_named() {
            std::env::set_var("SPAWNS_GLOBAL_SPAWNER", "drop");
            let handle = spawn(ready(()));
            let err = block_on(handle).unwrap_err();
            assert!(err.is_cancelled());
        }
    }
}
