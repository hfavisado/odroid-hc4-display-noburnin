# Agent notes

- This is a dependency-free Rust/Linux app. Main code is in `src/lib.rs` and `src/main.rs`.
- Config keys and defaults are in `odroid-hc4-display.conf`; see README for data sources.
- Validate code changes with `cargo fmt --check`, `cargo test`, and `cargo build --release`.
- Keep changes focused; no external crates are needed for the current I²C, sysfs, and command-based sources.
