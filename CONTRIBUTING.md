# Contributing to Wallr

Thank you for your interest in contributing to Wallr! Wallr is a native, GPU-accelerated Wayland wallpaper engine written in Rust.

---

## Development Environment Setup

### Prerequisites

You will need the Rust toolchain (edition 2024 / MSRV 1.85+), Wayland development headers, and FFmpeg development libraries (required by `ffmpeg-next` for video support) installed on your system.

#### Arch Linux
```bash
sudo pacman -S rustup wayland wayland-protocols pkg-config ffmpeg
```

#### Fedora
```bash
sudo dnf install rust cargo wayland-devel wayland-protocols-devel pkg-config ffmpeg-devel
```

#### Ubuntu / Debian
```bash
sudo apt install rustc cargo libwayland-dev wayland-protocols pkg-config libavcodec-dev libavformat-dev libavutil-dev libswscale-dev
```

### Getting Started

1. **Fork and Clone**:
   ```bash
   git clone https://github.com/YOUR_USERNAME/wallr.git
   cd wallr
   ```

2. **Build the Workspace**:
   ```bash
   cargo build
   ```

3. **Run Tests**:
   ```bash
   cargo test
   ```

---

## Workspace Structure

The project is structured as a Cargo workspace:

```
wallr/
├── Cargo.toml                  # Workspace manifest
├── wallr/                      # Binary crate (CLI frontend)
│   └── src/
│       └── main.rs             # Main CLI entrypoint & IPC client
├── wallr-core/                 # Library crate (Core Engine)
│   ├── src/
│   │   ├── animation/          # Animation spec parsing, timeline & uniform computation
│   │   ├── animated/           # Animated GIF decoding & wall-clock playback timing
│   │   ├── easing/              # Cubic-bezier and spring curves
│   │   ├── custom_effects/      # Sandboxed field validation/transpilation
│   │   ├── cache/              # Frame & package cache management
│   │   ├── cli/                # Clap CLI structures and commands
│   │   ├── config/             # Config loader, parser, paths & defaults
│   │   ├── daemon/             # Daemon event loop, Wayland layer-shell & IPC socket server
│   │   ├── ipc/                # Unix domain socket IPC protocol & messaging
│   │   ├── packages/           # Animation package registry, remote fetcher & dependency solver
│   │   ├── preview/            # Wallpaper preview renderer window
│   │   ├── renderer/           # wgpu GPU rendering pipeline
│   │   ├── shader/             # WGSL shader bindings and uniform layouts
│   │   ├── theme/              # Matugen, Wallust, Pywal, & hook dispatchers
│   │   ├── video/             # FFmpeg decoding & PTS playback scheduling
│   │   └── wallpaper/          # High-level engine coordinator & diagnostics (doctor)
│   └── shaders/                # WGSL shader source files
│       └── effects.wgsl        # Fragment transition effects
├── animations/                 # Built-in animation package templates
│   ├── apple/liquid.yaml
│   ├── dramatic/wipe-blur.yaml
│   ├── minimal/minimal.yaml
│   ├── pixel/retro.yaml
│   └── smooth/crossfade.yaml
```

---

## Code Quality Standards

We enforce strict Rust code quality standards:

1. **Formatting**: Always format your code with `cargo fmt`:
   ```bash
   cargo fmt --all --check
   ```

2. **Clippy Lints**: Ensure there are no warnings or errors reported by `cargo clippy`:
   ```bash
   cargo clippy --workspace --all-targets -- -D warnings
   ```

3. **Testing**: Add unit or integration tests for new functionality and verify all tests pass:
   ```bash
   cargo test --workspace
   ```

Non-trivial logic requires tests: config merge order, duration parsing, timeline
scheduling, easing math, effect validation, package cycle detection, custom
effect transpilation, GIF frame indexing, and video scheduling. Library errors use `thiserror`;
`anyhow` belongs at the binary boundary. Do not add unused dependencies, stubs,
`TODO` comments, or `unwrap()` calls to library code.

---

## How to Add a New GPU Effect

1. **WGSL Shader Implementation**:
   Add or modify fragment logic inside `wallr-core/shaders/effects.wgsl`. Effect selection is driven by `uniforms.effect_type`.

2. **Animation Spec Enum**:
   Define parameter structs and add the effect variant to the `Effect` enum in `wallr-core/src/animation/mod.rs`:
   ```rust
   #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
   pub struct MyCustomEffectParams { ... }

   pub enum Effect {
       // ...
       MyCustomEffect(MyCustomEffectParams),
   }
   ```

3. **Uniform Computation**:
   Update `compute_effect_uniforms` in `wallr-core/src/animation/mod.rs` to map parameters and progress into `EffectUniforms`.

4. **Validation & Testing**:
   Write tests for deserialization and uniform computation in `wallr-core/src/animation/mod.rs`. Test with `wallr validate <yaml>`.

---

## How to Add a New Theme Provider

1. **Implement Provider Dispatch**:
   Add a new variant to `ThemeProvider` enum in `wallr-core/src/config/mod.rs`.

2. **Implement Runner**:
   In `wallr-core/src/theme/mod.rs`, add a runner function (e.g. `run_my_theme_provider`) that spawns the executable with appropriate CLI flags.

3. **Update Availability Check**:
   Update `check_provider_available` in `wallr-core/src/theme/mod.rs` so `wallr doctor` can verify system binary presence.

---

## Pull Request Guidelines

1. **Create a Feature Branch**:
   ```bash
   git checkout -b feature/my-awesome-feature
   ```

2. **Commit Messages**:
   Write descriptive commit messages explaining *what* changed and *why*.

3. **Open Pull Request**:
   - Provide a concise summary of your changes.
   - Mention any related issues or discussions.
   - Ensure all CI checks (fmt, clippy, test) pass locally before requesting review.

---

## License

By contributing to Wallr, you agree that your contributions will be licensed under the project's [MIT License](LICENSE).
