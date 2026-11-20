# Build environment: Rust (GNU target) + MinGW via a junction without spaces in the path.
export PATH="$HOME/.cargo/bin:/g/games/MenmaLauncher/tools/mingw64/bin:$PATH"
# Build output on the internal SSD (the G: USB drive has been throwing I/O retries).
export CARGO_TARGET_DIR="C:/ProgramData/MenmaBuild/target"
