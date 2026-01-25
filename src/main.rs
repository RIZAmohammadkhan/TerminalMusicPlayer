#[cfg(not(target_os = "linux"))]
compile_error!("trix-player is Linux-only. Build on Linux (target_os=\"linux\").");

fn main() -> anyhow::Result<()> {
    trix_player::run()
}