//! Shared setup for the examples.

/// Install a `tracing` subscriber. The level comes from `RUST_LOG`
/// (for example `RUST_LOG=cafef=debug`) and defaults to `info`.
pub fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

/// Value following `--name` on the command line.
#[allow(dead_code)]
pub fn flag_value(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}
