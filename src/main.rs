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

fn main() -> anyhow::Result<()> {
    app::run()
}