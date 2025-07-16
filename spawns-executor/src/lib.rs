use async_executor::Executor;
use async_shutdown::ShutdownManager;
use futures::executor::{block_on as block_current_thread_on, LocalPool, LocalSpawner};
use futures::task::{FutureObj, Spawn as _};
use spawns_core::{enter, spawn, Spawn, Task};
use std::boxed::Box;
use std::future::Future;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::thread;

struct Spawner {
    spawner: LocalSpawner,
}

impl Spawn for Spawner {
    fn spawn(&self, task: Task) {
        let Task { future, .. } = task;
        self.spawner.spawn_obj(FutureObj::new(future)).unwrap()
    }
}

struct ExecutorSpawner<'a> {
    executor: &'a Executor<'static>,
}

impl<'a> ExecutorSpawner<'a> {
    fn new(executor: &'a Executor<'static>) -> Self {
        Self { executor }
    }
}

impl Spawn for ExecutorSpawner<'_> {
    fn spawn(&self, task: Task) {
        let Task { future, .. } = task;
        self.executor.spawn(Box::into_pin(future)).detach();
    }
}

/// Executor construct to block future until completion.
pub struct Blocking {
    parallelism: usize,
}

impl Blocking {
    /// Creates an executor to run future and its assistant tasks.
    ///
    /// # Notable behaviors
    /// * `0` means [thread::available_parallelism].
    /// * `1` behaves identical to [block_on].
    pub fn new(parallelism: usize) -> Self {
        Self { parallelism }
    }

    fn parallelism(&self) -> usize {
        match self.parallelism {
            0 => std::thread::available_parallelism().map_or(2, NonZeroUsize::get),
            n => n,
        }
    }

    fn run_until<T, F>(executor: &Executor<'static>, future: F) -> T
    where
        F: Future<Output = T> + Send + 'static,
    {
        let spawner = ExecutorSpawner::new(executor);
        let _scope = enter(&spawner);
        block_current_thread_on(executor.run(future))
    }

    /// Blocks current thread and runs given future until completion.
    ///
    /// All task will be cancelled sooner or later after return.
    ///
    /// Uses [spawn] to spawn assistant tasks.
    pub fn block_on<T: Send + 'static, F: Future<Output = T> + Send + 'static>(
        self,
        future: F,
    ) -> F::Output {
        let threads = self.parallelism();
        if threads == 1 {
            return block_on(future);
        }
        let executor = Arc::new(Executor::new());
        let shutdown = ShutdownManager::new();
        let shutdown_signal = shutdown.wait_shutdown_triggered();
        (2..=threads).for_each(|i| {
            thread::Builder::new()
                .name(format!("spawns-executor-{i}/{threads}"))
                .spawn({
                    let executor = executor.clone();
                    let shutdown_signal = shutdown_signal.clone();
                    move || Self::run_until(&executor, shutdown_signal)
                })
                .unwrap();
        });
        let _shutdown_on_drop = shutdown.trigger_shutdown_token(());
        Self::run_until(&executor, future)
    }
}

/// Blocks current thread and runs given future until completion.
///
/// All task will be cancelled after return.
///
/// Uses [spawn] to spawn assistant tasks.
pub fn block_on<T: Send + 'static, F: Future<Output = T> + Send + 'static>(future: F) -> F::Output {
    let mut pool = LocalPool::new();
    let spawner = Spawner {
        spawner: pool.spawner(),
    };
    let _scope = enter(&spawner);
    pool.run_until(spawn(future)).unwrap()
}

#[cfg(test)]
mod tests {
    use super::{block_current_thread_on, block_on, Blocking};
    use spawns_core as spawns;

    mod echo {
        // All this module are runtime agnostic.
        use async_net::*;
        use futures_lite::io;
        use futures_lite::prelude::*;
        use spawns_core::{spawn, TaskHandle};

        async fn echo_stream(stream: TcpStream) {
            let (reader, writer) = io::split(stream);
            let _ = io::copy(reader, writer).await;
        }

        async fn echo_server(listener: TcpListener) {
            let mut echos = vec![];
            loop {
                let (conn, _addr) = listener.accept().await.unwrap();
                echos.push(spawn(echo_stream(conn)).attach());
            }
        }

        async fn start_echo_server() -> (u16, TaskHandle<()>) {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let handle = spawn(echo_server(listener));
            (port, handle.attach())
        }

        pub async fn echo_one(data: &[u8]) -> Vec<u8> {
            let (port, _server_handle) = start_echo_server().await;
            let mut stream = TcpStream::connect(format!("127.0.0.1:{port}"))
                .await
                .unwrap();
            stream.write_all(data).await.unwrap();
            stream.close().await.unwrap();
            let mut buf = vec![];
            stream.read_to_end(&mut buf).await.unwrap();
            buf
        }
    }

    #[test]
    fn block_on_current_thread() {
        let msg = b"Hello! Current Thread Executor!";
        let result = block_on(echo::echo_one(msg));
        assert_eq!(&result[..], msg);
    }

    #[test]
    fn block_on_multi_thread() {
        let msg = b"Hello! Multi-Thread Executor!";
        let result = Blocking::new(4).block_on(echo::echo_one(msg));
        assert_eq!(&result[..], msg);
    }

    #[test]
    fn task_cancelled_after_main_return_current_thread() {
        use async_io::Timer;
        use std::time::Duration;
        #[allow(clippy::async_yields_async)]
        let handle = block_on(async {
            spawns::spawn(async { Timer::after(Duration::from_secs(30)).await })
        });
        let err = block_current_thread_on(handle).unwrap_err();
        assert!(err.is_cancelled());
    }

    #[test]
    fn task_cancelled_after_main_return_multi_thread() {
        use async_io::Timer;
        use std::time::Duration;
        #[allow(clippy::async_yields_async)]
        let handle = Blocking::new(4).block_on(async {
            spawns::spawn(async { Timer::after(Duration::from_secs(30)).await })
        });
        let err = block_current_thread_on(handle).unwrap_err();
        assert!(err.is_cancelled());
    }
}
