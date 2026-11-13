/// release-please bumps the Python package's `__version__`; reading it at
/// build time keeps the Rust server on the same number without its own bump.
const PYTHON_INIT: &str = include_str!("../../src/activsync/__init__.py");

pub static VERSION: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    PYTHON_INIT
        .lines()
        .find_map(|line| line.trim().strip_prefix("__version__ = \""))
        .and_then(|rest| rest.split('"').next())
        .unwrap_or("0.0.0")
        .to_string()
});

/// Dev images name their branch build in `ACTIVSYNC_BUILD`.
pub fn display_version() -> String {
    std::env::var("ACTIVSYNC_BUILD")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| VERSION.clone())
}
