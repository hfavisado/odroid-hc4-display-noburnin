# Agent notes

- This is a dependency-free Rust/Linux app. Main code is in `src/lib.rs` and `src/main.rs`.
- Config keys and defaults are in `odroid-hc4-display.conf`; see README for data sources.
- Validate code changes with `cargo fmt --check`, `cargo test`, and `cargo build --release`.
- On the current development machine, Nix and devenv are available, but this repository has no devenv configuration. If Cargo is not on `PATH`, use the Nix toolchain instead of installing rustup:
  `nix shell nixpkgs#cargo nixpkgs#rustc nixpkgs#rustfmt -c sh -c 'cargo fmt --check && cargo test && cargo build --release'`
- Keep changes focused; no external crates are needed for the current I²C, sysfs, and command-based sources.
- After changing rendering (font, scaling, scroll), regenerate the README preview: `nix shell nixpkgs#cargo nixpkgs#rustc nixpkgs#rustfmt -c cargo run --release -- --preview docs/preview.gif` (or use Cargo directly if available).
