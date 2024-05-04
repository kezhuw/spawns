//! `spawns` proposes thread context task spawner to ease async runtime agnostic coding.
//!
//! It introduces few concepts for async runtimes to setup thread context task spawner:
//! * `Spawn` trait to spawn task.
//! * `enter()` to enter spawn scope.
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
//! To cooperate with existing async runtimes, it provides features to inject spawners for them.
//! * `tokio`: uses `tokio::runtime::Handle::try_current()` to detect thread local `tokio` runtime handle.
//! * `smol`: uses `smol::spawn` to spawn task in absent of thread local spawners.
//! * `async-global-executor`: uses `async_global_executor::spawn` to spawn task in absent of thread local spawners.
//!
//! Be aware that, `smol` and `async-global-executor` are not compatible as they both blindly spawn
//! tasks. [spawn()] will panic if there is no thread local spawners but multiple global spawners.

pub use spawns_core::*;
pub use spawns_executor::*;

#[cfg(feature = "spawns-compat")]
static _LINK_COMPAT: () = spawns_compat::__linkme_const();
