#[cfg(not(target_os = "linux"))]
compile_error!("trix-player is Linux-only. Build on Linux (target_os=\"linux\").");

mod app;
mod audio;
mod config;
mod library;
mod meta;
mod player;
mod term;
mod ui;
mod util;

/// Runs the terminal music player.
///
/// The binary entrypoint (`src/main.rs`) delegates to this so the codebase can be
/// structured like a normal Rust library.
pub fn run() -> anyhow::Result<()> {
    app::run()
}
