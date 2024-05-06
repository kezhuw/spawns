# Spawns

[![crates.io](https://img.shields.io/crates/v/spawns)](https://crates.io/crates/spawns)
[![github-ci](https://github.com/kezhuw/spawns/actions/workflows/ci.yml/badge.svg?event=push)](https://github.com/kezhuw/spawns/actions)
[![codecov](https://codecov.io/gh/kezhuw/spawns/graph/badge.svg?token=qSY9PdISfH)](https://codecov.io/gh/kezhuw/spawns)
[![docs.rs](https://img.shields.io/docsrs/spawns)](https://docs.rs/spawns)
[![Apache-2.0](https://img.shields.io/github/license/kezhuw/spawns)](LICENSE)

Thread context task spawner for Rust to ease async runtime agnostic coding.

## Motivation

Currently, Rust does not have a standard async runtime. This exposes us a dilemma to choose one and makes creating runtime agnostic library pretty hard. The most challenging thing we have to face is how to spawn task ?

`spawns` proposes a thread context task spawner for Rust `std` and async runtimes. Once delivered, we are able to spawn tasks in runtime agnostic manner. Together with other runtime agnostic io, timer, channel and etc. crates, we are capable to write runtime agnostic code easily.


## API for async runtimes

```rust
/// Thin wrapper around task to accommodate possible new members.
#[non_exhaustive]
pub struct Task {
    pub id: Id,
    pub name: Name,
    pub future: Box<dyn Future<Output = ()> + Send + 'static>,
}

/// Trait to spawn task.
pub trait Spawn {
    fn spawn(&self, task: Task);
}

/// Scope where tasks are [spawn]ed through given [Spawn].
pub struct SpawnScope<'a> {}

/// Enters a scope where new tasks will be [spawn]ed through given [Spawn].
pub fn enter(spawner: &dyn Spawn) -> SpawnScope<'_>;
```

Async runtimes have to do two things to accommodate for other runtime agnostic API.

1. Implements `Spawn` to spawn asynchronous task.
2. Calls `enter` in all executor threads.

## API for clients
```rust
impl<T> JoinHandle<T> {
    /// Gets id of the associated task.
    pub fn id(&self) -> Id {}

    /// Cancels associated task with this handle.
    ///
    /// Cancellation is inherently concurrent with task execution. Currently, there is no guarantee
    /// about promptness, the task could even run to complete normally after cancellation.
    pub fn cancel(&self) { }

    /// Attaches to associated task to gain cancel on [Drop] permission.
    pub fn attach(self) -> TaskHandle<T> { }
}

impl<T> Future for JoinHandle<T> {
    type Output = Result<T, JoinError>;
}

/// Spawns a new task.
///
/// # Panics
/// 1. Panic if no spawner.
/// 2. Panic if [Spawn::spawn] panic.
pub fn spawn<T, F>(f: F) -> JoinHandle<T>
where
    F: Future<Output = T> + Send + 'static,
    T: Send + 'static;
```

The API is capable to spawn, join and cancel tasks as what `tokio`, `smol` and `async-std` do.

## Concerns
1. Boxing ? Yes, it needs `GlobalAlloc`.
2. Boxing even the entry future ? No, but `try_id()` will return `None`. I guess we could provides function to wrap a bit.
3. `no_std` ? No, it needs `thread_local!` currently. We can move this to [`#[thread_local]`](https://github.com/rust-lang/rust/issues/29594) once stabilized.
4. `spawn_local` for `!Send` future ? No, at least for now. I saw only `async-global-executor` is capable to `spawn_local` freely. Personally, I think it is Rust's responsibility to not treat futures owning `!Send` as `!Send`. This way there are little chance for us to create `!Send` futures. For futures that capturing `!Send` in first place and storing thread local `!Send`, they need current thread executor.

## Packages
1. [spawns-core][] provides `Spawn` and `enter()` for async runtimes to setup thread context task spawner.
2. [spawns-compat][] provides compatibility for `tokio`, `smol` and `async-global-executor`(which is used by `async-std`) through feature gates.
3. [spawns-executor][] provides full functional `block_on` with both current thread executor and multi-thread executor.
4. [spawns][] exports all above packages including feature gates `tokio`, `smol` and `async-global-executor`. In addition, it provides feature gate `executor` to include `spawns-executor`.

## Examples
See [examples](examples/). A minimum runtime agnostic echo server is listed here for demonstration.

```rust
use async_net::*;
use futures_lite::io;

pub async fn echo_server(port: u16) {
    let listener = TcpListener::bind(("127.0.0.1", port)).await.unwrap();
    println!("Listen on port: {}", listener.local_addr().unwrap().port());
    let mut echos = vec![];
    let mut id_counter = 0;
    loop {
        let (stream, remote_addr) = listener.accept().await.unwrap();
        id_counter += 1;
        let id = id_counter;
        let handle = spawns::spawn(async move {
            eprintln!("{:010}[{}]: serving", id, remote_addr);
            let (reader, writer) = io::split(stream);
            match io::copy(reader, writer).await {
                Ok(_) => eprintln!("{:010}[{}]: closed", id, remote_addr),
                Err(err) => eprintln!("{:010}[{}]: {:?}", id, remote_addr, err),
            }
        })
        .attach();
        echos.push(handle);
    }
}
```

All you have to do for it to be function is setting up thread context task spawner.

## License
[Apache-2.0](LICENSE)

[spawns]: https://docs.rs/spawns
[spawns-core]: https://docs.rs/spawns-core
[spawns-compat]: https://docs.rs/spawns-compat
[spawns-executor]: https://docs.rs/spawns-executor
