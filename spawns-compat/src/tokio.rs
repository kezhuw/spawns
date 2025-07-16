use linkme::distributed_slice;
use spawns_core::{Compat, Task, COMPATS};
use std::boxed::Box;

#[distributed_slice(COMPATS)]
pub static TOKIO: Compat = Compat::Local(tokio_local);

fn tokio_spawn(task: Task) {
    let Task { future, .. } = task;
    let handle = tokio::runtime::Handle::current();
    handle.spawn(Box::into_pin(future));
}

fn tokio_local() -> Option<fn(Task)> {
    tokio::runtime::Handle::try_current()
        .ok()
        .map(|_| tokio_spawn as fn(Task))
}

#[cfg(test)]
#[cfg(feature = "tokio")]
mod tests {
    use spawns_core::*;

    #[tokio::test]
    async fn spawn_one() {
        let handle = spawn(async { 5 });
        let result = handle.await.unwrap();
        assert_eq!(result, 5);
    }

    #[tokio::test]
    async fn spawn_cascading() {
        #[allow(clippy::async_yields_async)]
        let handle = spawn(async { spawn(async { id() }) });
        let handle = handle.await.unwrap();
        let id = handle.id();
        assert_eq!(handle.await.unwrap(), id);
    }

    #[tokio::test]
    async fn spawn_interleaving() {
        #[allow(clippy::async_yields_async)]
        let handle = spawn(async { tokio::spawn(async { spawn(async { id() }) }) });
        let handle = handle.await.unwrap().await.unwrap();
        let id = handle.id();
        assert_eq!(handle.await.unwrap(), id);
    }

    #[tokio::test]
    async fn spawn_into_tokio() {
        #[allow(clippy::async_yields_async)]
        let handle = spawn(async { tokio::spawn(async { try_id() }) });
        let handle = handle.await.unwrap();
        assert_eq!(handle.await.unwrap(), None);
    }
}
