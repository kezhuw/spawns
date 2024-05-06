//! `spawns` proposes thread context task spawner to ease async runtime agnostic coding.
//!
//! It introduces few concepts for async runtimes to setup thread context task spawner:
//! * [Spawn] trait to spawn task.
//! * [enter()] to enter spawn scope.
//!
//! With above, `spawns` provides [spawn()] and [JoinHandle] to spawn and join tasks.
//!
//! Below is a minimum runtime agnostic echo server. All you have to do for it to be function is
//! setting up thread context task spawner.
//!
//! ```no_run
//! use async_net::*;
//! use futures_lite::io;
//!
//! pub async fn echo_server(port: u16) {
//!     let listener = TcpListener::bind(("127.0.0.1", port)).await.unwrap();
//!     println!("Listen on port: {}", listener.local_addr().unwrap().port());
//!     let mut echos = vec![];
//!     let mut id_counter = 0;
//!     loop {
//!         let (stream, remote_addr) = listener.accept().await.unwrap();
//!         id_counter += 1;
//!         let id = id_counter;
//!         let handle = spawns::spawn(async move {
//!             eprintln!("{:010}[{}]: serving", id, remote_addr);
//!             let (reader, writer) = io::split(stream);
//!             match io::copy(reader, writer).await {
//!                 Ok(_) => eprintln!("{:010}[{}]: closed", id, remote_addr),
//!                 Err(err) => eprintln!("{:010}[{}]: {:?}", id, remote_addr, err),
//!             }
//!         })
//!         .attach();
//!         echos.push(handle);
//!     }
//! }
//! ```
//!
//! ## Compatibility with existing async runtimes
//!
//! This is an open world, there might be tens async runtimes. `spawns` provides features to inject
//! spawners for few.
//!
//! * `tokio`: uses `tokio::runtime::Handle::try_current()` to detect thread local `tokio` runtime handle.
//! * `smol`: uses `smol::spawn` to spawn task in absent of thread local spawners.
//! * `async-global-executor`: uses `async_global_executor::spawn` to spawn task in absent of thread local spawners.
//!
//! For other async runtimes, one could inject [Compat]s to [static@COMPATS] themselves.
//!
//! Noted that, all those compatibility features, injections should only active on tests and
//! binaries. Otherwise, they will be propagated to dependents with unnecessary dependencies.
//!
//! ## Dealing with multiple global executors
//! Global executor cloud spawn task with no help from thread context. But this exposes us an
//! dilemma to us, which one to use if there are multiple global executors present ? By default,
//! `spawns` randomly chooses one and stick to it to spawn tasks in absent of thread context
//! spawners. Generally, this should be safe as global executors should be designed to spawn
//! everywhere. If this is not the case, one could use environment variable `SPAWNS_GLOBAL_SPAWNER`
//! to specify one. As a safety net, feature `panic-multiple-global-spawners` is provided to panic
//! if there are multiple global candidates.

pub use spawns_core::*;

#[cfg(feature = "executor")]
pub use spawns_executor::*;

#[cfg(feature = "spawns-compat")]
extern crate spawns_compat;
