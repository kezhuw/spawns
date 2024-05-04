use linkme::distributed_slice;
use spawns_core::{Compat, Task, COMPATS};
use std::boxed::Box;

#[distributed_slice(COMPATS)]
pub static ASYNC_GLOBAL_EXECUTOR: Compat = Compat::Global(async_global);

fn async_global(task: Task) {
    let Task { future, .. } = task;
    async_global_executor::spawn(Box::into_pin(future)).detach()
}

#[cfg(test)]
#[cfg(feature = "async-global-executor")]
#[cfg(not(feature = "smol"))]
mod tests {
    use spawns_core::*;

    #[test]
    fn spawn_one() {
        async_std::task::block_on(async {
            let handle = spawn(async { id() });
            let id = handle.id();
            assert_eq!(handle.await.unwrap(), id);
        });
    }

    #[test]
    fn spawn_cascading() {
        async_std::task::block_on(async {
            let handle = spawn(async { spawn(async { id() }) });
            let handle = handle.await.unwrap();
            let id = handle.id();
            assert_eq!(handle.await.unwrap(), id);
        });
    }

    #[test]
    fn spawn_interleaving() {
        async_std::task::block_on(async move {
            let handle = spawn(async { async_std::task::spawn(async { spawn(async { id() }) }) });
            let handle = handle.await.unwrap().await;
            let id = handle.id();
            assert_eq!(handle.await.unwrap(), id);
        });
    }

    #[test]
    fn spawn_into_smol() {
        async_std::task::block_on(async move {
            let handle = spawn(async { async_std::task::spawn(async { try_id() }) });
            let handle = handle.await.unwrap();
            assert_eq!(handle.await, None);
        });
    }
}
