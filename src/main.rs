#[cfg(not(target_os = "linux"))]
compile_error!("trix-player is Linux-only. Build on Linux (target_os=\"linux\").");

mod app;
mod audio;
mod config;
mod library;
mod meta;
mod output;
mod player;
mod shell;
mod term;
mod ui;
mod util;
mod volume;

fn main() -> anyhow::Result<()> {
    app::run()
}