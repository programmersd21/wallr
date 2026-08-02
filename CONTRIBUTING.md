# Contributing to Wallr

Wallr is a native, GPU-accelerated Wayland wallpaper engine written in Rust. This document covers environment setup, code standards, and how to add new effects and theme providers.

## Setup

Requires the Rust toolchain (edition 2024, MSRV 1.85+), Wayland development headers, and FFmpeg development libraries (used by `ffmpeg-next` for video support).

```bash
# Arch
sudo pacman -S rustup wayland wayland-protocols pkg-config ffmpeg

# Fedora
sudo dnf install rust cargo wayland-devel wayland-protocols-devel pkg-config ffmpeg-devel

# Ubuntu/Debian
sudo apt install rustc cargo libwayland-dev wayland-protocols pkg-config libavcodec-dev libavformat-dev libavutil-dev libswscale-dev
```

```bash
git clone https://github.com/programmersd21/wallr.git
cd wallr
cargo build
cargo test
```

## Workspace layout

```
wallr/
├── Cargo.toml                  # Workspace manifest
├── wallr/                      # Binary crate (CLI frontend)
│   └── src/
│       └── main.rs             # CLI entrypoint & IPC client
├── wallr-core/                 # Core engine library
│   ├── src/
│   │   ├── animation/          # Animation spec parsing, timeline & uniform computation
│   │   ├── animated/           # GIF decoding & wall-clock playback timing
│   │   ├── easing/             # Cubic-bezier and spring curves
│   │   ├── custom_effects/     # Sandboxed field validation/transpilation
│   │   ├── cache/              # Frame & package cache management
│   │   ├── cli/                # Clap CLI structures and commands
│   │   ├── config/             # Config loader, parser, paths & defaults
│   │   ├── daemon/             # Daemon event loop, layer-shell & IPC socket server
│   │   ├── ipc/                # Unix socket IPC protocol & messaging
│   │   ├── packages/           # Animation package registry, fetcher & dependency solver
│   │   ├── preview/            # Wallpaper preview window
│   │   ├── renderer/           # wgpu rendering pipeline
│   │   ├── shader/             # WGSL shader bindings and uniform layouts
│   │   ├── theme/              # Matugen, Wallust, Pywal, & hook dispatchers
│   │   ├── video/              # FFmpeg decoding & PTS playback scheduling
│   │   └── wallpaper/          # Engine coordinator & diagnostics (doctor)
│   └── shaders/
│       └── effects.wgsl        # Fragment transition effects
├── animations/                 # Built-in animation package templates
│   ├── apple/liquid.yaml
│   ├── dramatic/wipe-blur.yaml
│   ├── minimal/minimal.yaml
│   ├── pixel/retro.yaml
│   └── smooth/crossfade.yaml
```

## Code standards

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Non-trivial logic needs tests: config merge order, duration parsing, timeline scheduling, easing math, effect validation, package cycle detection, custom effect transpilation, GIF frame indexing, video scheduling.

Library errors use `thiserror`. `anyhow` stays at the binary boundary. No unused dependencies, stub functions, `TODO` comments, or `unwrap()` in library code.

## Adding a GPU effect

1. Add the fragment logic to `wallr-core/shaders/effects.wgsl`. Effect selection is driven by `uniforms.effect_type`.

2. Define the parameter struct and add a variant to `Effect` in `wallr-core/src/animation/mod.rs`:

   ```rust
   #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
   pub struct MyCustomEffectParams { ... }

   pub enum Effect {
       // ...
       MyCustomEffect(MyCustomEffectParams),
   }
   ```

3. Map the parameters and progress into `EffectUniforms` in `compute_effect_uniforms` (same file).

4. Add deserialization and uniform-computation tests. Verify with `wallr validate <yaml>`.

## Adding a theme provider

1. Add a variant to `ThemeProvider` in `wallr-core/src/config/mod.rs`.
2. Add a runner (e.g. `run_my_theme_provider`) in `wallr-core/src/theme/mod.rs` that spawns the executable with the right flags.
3. Update `check_provider_available` in the same file so `wallr doctor` can detect it.

## Pull requests

```bash
git checkout -b feature/my-feature
```

Write commit messages that explain what changed and why. In the PR description, summarize the change and link any related issues. Run `fmt`, `clippy`, and `test` locally before requesting review.

## License

Contributions are licensed under the project's [MIT License](LICENSE).
