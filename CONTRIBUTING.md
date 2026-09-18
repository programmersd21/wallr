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
├── wallr/                      # Binary crate (CLI frontend, single `wallr` binary)
│   └── src/
│       └── main.rs             # CLI entrypoint & IPC client
├── wallr-common/               # Shared protocol library (no GPU/Wayland/decoding)
│   └── src/
│       ├── cli.rs              # Clap CLI shapes
│       ├── config.rs           # Config schema, parsing, paths
│       ├── effect.rs           # Transition effects & uniform computation
│       ├── ipc.rs              # IPC command/response vocabulary & validation
│       └── types.rs            # ScalingMode, ThemeProvider, GpuSelection
├── wallr-core/                 # Core engine library
│   ├── src/
│   │   ├── animation/          # Transition effects & uniform computation
│   │   ├── animated/           # GIF decoding & wall-clock playback timing
│   │   ├── cli/                # Clap CLI structures and commands
│   │   ├── config/             # Config loader, parser, paths & defaults
│   │   ├── daemon/             # Daemon event loop, layer-shell & IPC socket server
│   │   ├── ipc/                # Unix socket IPC protocol & messaging
│   │   ├── renderer/           # wgpu rendering pipeline
│   │   ├── shader/             # WGSL shader bindings and uniform layouts
│   │   ├── theme/              # Matugen, Wallust, Pywal, & hook dispatchers
│   │   ├── video/              # FFmpeg decoding & PTS playback scheduling
│   │   └── wallpaper/          # Engine coordinator & diagnostics (doctor)
│   └── shaders/
│       └── effects.wgsl        # Fragment transition effects
```

## Code standards

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

For performance changes, build the release binary and run the live benchmark
from a Wayland session:

```bash
cargo build --workspace --release
scripts/benchmark-wallpapers.sh
```

Record compositor, GPU, output resolution, Wallr commit, competitor version,
and workload with any result. Avoid universal performance claims from a single
machine; verify idle, repeated-request, distinct-image, and animated/video
cases separately.

Non-trivial logic needs tests: config defaults, duration parsing, effect uniform mapping, GIF frame indexing, video scheduling.

Library errors use `thiserror`. `anyhow` stays at the binary boundary. No unused dependencies, stub functions, `TODO` comments, or `unwrap()` in library code.

## Adding a transition effect

1. Add the fragment logic to `wallr-core/shaders/effects.wgsl`. Effect selection is driven by `uniforms.effect_type`; append the next free index.

2. Define the parameter struct and add a variant to `Effect` in `wallr-core/src/animation/mod.rs`:

   ```rust
   #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
   pub struct MyEffectParams { ... }

   pub enum Effect {
       // ...
       MyEffect(MyEffectParams),
   }
   ```

3. Map the parameters and progress into `EffectUniforms` in `compute_effect_uniforms` (same file). Extend the `effect_types_match_shader_arms` test so the numbering can never drift from the shader.

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
