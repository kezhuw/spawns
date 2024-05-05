//! `spawns-compat` provides functions to find async runtimes for spawning task.
//!
//! It uses [linkme] to inject these functions to `spawns-core`. In some platforms, this crate may
//! not be linked to binary finally. So, you may have to speak explicitly about this with `extern
//! crate spawns_compat`. You could also use it with `spawns-core` through `spawns` which does so
//! for you. Besides this, in macOS, you may have to put below to your `Cargo.toml`.
//!
//! ```toml
//! [profile.dev]
//! lto = "thin"
//! ```
//!
//! See for details:
//! * <https://stackoverflow.com/questions/29403920/whats-the-difference-between-use-and-extern-crate/29404692#29404692>
//! * <https://github.com/dtolnay/linkme/issues/31>
//! * <https://github.com/dtolnay/linkme/issues/61>

#[cfg(feature = "smol")]
mod smol;
#[cfg(feature = "tokio")]
mod tokio;

#[cfg(feature = "async-global-executor")]
mod async_global_executor;

#[cfg(test)]
mod tests {
    #[test]
    #[cfg(all(feature = "async-global-executor", feature = "smol"))]
    #[should_panic(expected = "multiple global spawners")]
    fn multiple_global() {
        spawns_core::spawn(async move {});
    }
}
