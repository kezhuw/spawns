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
