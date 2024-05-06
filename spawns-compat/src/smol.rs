use linkme::distributed_slice;
use spawns_core::{Compat, Task, COMPATS};
use std::boxed::Box;

#[distributed_slice(COMPATS)]
pub static SMOL: Compat = Compat::NamedGlobal {
    name: "smol",
    spawn: smol_global,
};

fn smol_global(task: Task) {
    let Task { future, .. } = task;
    smol::spawn(Box::into_pin(future)).detach()
}

#[cfg(test)]
#[cfg(feature = "smol")]
#[cfg(not(feature = "async-global-executor"))]
mod tests {
    use futures_lite::future;
    use spawns_core::*;

    #[test]
    fn spawn_one() {
        future::block_on(async {
            let handle = spawn(async { id() });
            let id = handle.id();
            assert_eq!(handle.await.unwrap(), id);
        });
    }

    #[test]
    fn spawn_cascading() {
        future::block_on(async {
            let handle = spawn(async { spawn(async { id() }) });
            let handle = handle.await.unwrap();
            let id = handle.id();
            assert_eq!(handle.await.unwrap(), id);
        });
    }

    #[test]
    fn spawn_interleaving() {
        future::block_on(async move {
            let handle = spawn(async { smol::spawn(async { spawn(async { id() }) }) });
            let handle = handle.await.unwrap().await;
            let id = handle.id();
            assert_eq!(handle.await.unwrap(), id);
        });
    }

    #[test]
    fn spawn_into_smol() {
        future::block_on(async move {
            let handle = spawn(async { smol::spawn(async { try_id() }) });
            let handle = handle.await.unwrap();
            assert_eq!(handle.await, None);
        });
    }
}
