# Wallr Architecture Overview

`wallr` is built from the ground up as a native Wayland application in Rust. It does not rely on third-party wallpaper daemons or external display tools.

```
┌─────────────────────────────────────────────────────────┐
│                     wallr CLI                           │
│     (Clap parsing, daemon auto-start, IPC client)       │
└────────────────────────────┬────────────────────────────┘
                             │ Unix Domain Socket ($XDG_RUNTIME_DIR/wallr.sock)
                             ▼
┌─────────────────────────────────────────────────────────┐
│                    wallr daemon                         │
│                                                         │
│  ┌───────────────────┐        ┌──────────────────────┐  │
│  │ Wayland Event     │        │ wgpu Renderer        │  │
│  │ Loop (SCTK 0.19)  ├───────►│ Pipeline             │  │
│  │ (Layer Shell)     │        │ (WGSL Shaders)       │  │
│  └───────────────────┘        └──────────┬───────────┘  │
└──────────────────────────────────────────┼──────────────┘
                                           │ Hardware Surface Present
                                           ▼
┌─────────────────────────────────────────────────────────┐
│               Wayland Compositor                        │
│          (Hyprland / Sway / Niri / KWin)                │
└─────────────────────────────────────────────────────────┘
```

---

## Key Subsystems

### 1. Native Wayland Layer Shell (`smithay-client-toolkit`)
- **Protocol**: `wlr-layer-shell-unstable-v1`
- **Layer**: `Layer::Background`
- **Behavior**: Binds to Wayland display server, registers a full-screen background surface per output, handles `scale_factor_changed` and `configure` events dynamically, and sets `wl_surface.set_buffer_scale(scale_factor)` for crisp 1:1 physical pixel rendering on 4K and HiDPI displays.
- **Per-monitor**: Each Wayland output gets its own `LayerSurface`, wgpu `Surface`, and `RenderState`, stored in a `HashMap<String, Arc<Mutex<RenderState>>>` keyed by output name (e.g. `DP-1`, `HDMI-A-1`). Output names are resolved via `wl_output` v4; compositors that don't provide names fall back to make/model or a generated ID. The daemon performs 5 roundtrips at startup to ensure all outputs are discovered, even on compositors that deliver output events lazily.

### 2. GPU Rendering Pipeline (`wgpu`)
- **Backend**: Vulkan / OpenGL / Metal (via `wgpu` abstraction)
- **Shader Pipeline**: Single-pass WGSL shader (`effects.wgsl`)
- **Uniform Buffer**: Tracks separate old/new image aspect ratios, screen resolution, animation progress (`0.0..1.0`), active effect type index (`fade`, `blur`, `wipe`, `slide`, `zoom`, `pixelate`, `ripple`, `dissolve`, `wave`, `grow`, `outer`), effect parameters (`param_a` to `param_d`), effect origin (`origin`), travel direction (`direction`), easing mode (`easing`: `0` linear, `1` ease-in, `2` ease-out, `3` ease-in-out), and scaling mode (`scaling_mode`: `0` fill, `1` fit, `2` stretch, `3` center, `4` tile). Struct is 80 bytes (`Vec2`-aligned, size padded for WGSL uniform layout).
- **Aspect Correction**: Computes aspect-ratio scaling directly inside the fragment shader, avoiding CPU-side cropping or image scaling overhead. Five scaling modes are supported: `fill` (cover, crops to fill screen), `fit` (contain, letterbox/pillarbox), `stretch` (ignores aspect ratio), `center` (1:1 centered), and `tile` (repeat). The mode is passed to the GPU via a uniform and applied per-pixel in the `scale_uv` function. Circular effects (`grow`, `outer`, `ripple`) additionally convert UVs into pixel-aspect-corrected space before taking `distance()`, so expanding rings are true circles on any monitor, never ovals.
- **Stable Image Registration**: The old and new source textures each keep their own immutable `fill` crop for the whole transition. Effects animate blend values and reveal masks in screen space; they do not translate or rescale the wallpaper texture. This prevents the visible “jump” that occurs when images with different source dimensions are changed mid-transition.
- **Smoothness**: Every transition is eased with a configurable curve (`linear` / `ease_in` / `ease_out` / `ease_in_out`, default smoothstep cubic ease-in-out) and rendered one frame per vsync. The daemon presents with `PresentMode::Fifo`, so `get_current_texture()` blocks until the previous frame is displayed, pacing animations to the monitor refresh rate. Progress is derived from wall-clock time rather than a frame counter, so a transition lasts exactly its configured `duration` on any refresh rate (frame-count pacing would run too fast on high-refresh panels and too slow on low ones). The ease-in-out tail keeps visible motion almost to the last frame, so a blur radius or crossfade never appears to stagger to a halt before the transition finishes.
- **Non-blocking transitions**: The daemon commits the new wallpaper state immediately and renders the visual transition on a detached background task, serialized by a render lock. `wallr set` returns as soon as the image is committed and themed, never waiting on GPU presents. If the compositor stops presenting (monitor off, suspend), the render task parks inside the present without freezing the IPC loop, and later transitions simply queue behind it.
- **Live wallpapers**: when the committed file is an animated GIF, `AnimatedImage` decodes every frame once at load time. If the raw RGBA total fits the 256MB budget, frames are stored as-is and playback is a memcpy per frame; larger animations are stored as zstd-compressed streams (roughly 30:1) and decompressed through a persistent `zstd::bulk::Decompressor` context, so neither path ever re-decodes the source file during looping playback. The first frame becomes the transition's incoming texture, and when the transition ends the render task switches to playback: it computes the absolute wall-clock boundary of the next frame (`frame_start(index+1)` plus whole-loop offsets, so pacing survives animation wrap-around) and presents at that deadline, uploading the next frame into an idle double-buffered texture during the sleep via a mapped staging buffer (copied with `copy_buffer_to_texture`; unaligned widths fall back to `update_texture`). A playback generation counter is bumped on every commit, so a queued playback loop stops itself the moment a newer wallpaper supersedes it. Static images skip playback entirely and present a single frame, and the preview window follows the same flow.
- **Video playback**: `video::VideoPlayback` decodes MP4/WebM/MKV with FFmpeg, selecting a hardware decoder (VAAPI, NVDEC) when available and falling back to software. Frames are delivered on PTS timing through a small bounded queue and uploaded with the same texture pipeline; `wallpaper.loop_video` restarts the stream at EOF for seamless looping. `wallr ipc pause/resume/seek/info` control playback, and the video path is disabled for static images.
- **Previous-frame compositing**: The daemon keeps the previous decoded wallpaper texture as the outgoing source and reveals the new texture over it. The persisted last wallpaper is restored at daemon startup, so a restart also has a real outgoing frame. The preview window reads the same persisted path and uses the last applied wallpaper as its outgoing frame, falling back to a solid black frame only when nothing was ever applied (or when the outgoing image is the same file as the incoming one).

### 2b. Animation → Uniforms Path
Every transition (from a YAML package, `wallr set --effect ...`, or the preview window) resolves to a single `animation::Effect` value, which `compute_effect_uniforms(effect, progress)` converts into an `EffectUniforms` struct (effect type, eased progress, `param_a` to `param_d`, origin, direction, easing mode). The daemon and preview both feed this into `Renderer::render_frame`, so CLI flags, YAML packages, and previews share one identical code path:

```
CLI --effect/--origin/--angle/--easing/...   ┐
YAML animation package (first effect)        ├─► Effect ─► compute_effect_uniforms ─► EffectUniforms ─► WGSL
PreviewWindow (same Effect type)             ┘

### 3. Daemon & Unix IPC Server (`tokio`)
- **Socket**: `$XDG_RUNTIME_DIR/wallr.sock` (configurable)
- **Protocol**: JSON Line protocol over `tokio::net::UnixListener`
- **State Machine**: Holds persistent layer-shell surface handle and previous/current texture memory. Accepts IPC commands (`Pause`, `Resume`, `Reload`, `Preview`, `Stop`, `Status`, `Info`, `Seek`) without recreating Wayland windows. The `Preview` command carries a full serialized `animation::Effect` (name + all parameters), so every effect/position/easing combination is expressible over the wire. `Stop` removes the socket file before the process exits, so a stale socket never survives a clean shutdown.
- **Crate Layout**: `src/animation/` also provides `effect_from_name`, `effect_names`, `origin_from_preset`, and `apply_effect_overrides`, the shared helpers the CLI uses to translate `--effect/--origin/--angle/--easing/--direction/--from/--to/...` flags into an `Effect`.

### 4. Color Theme Pipeline (`theme`)
- **Providers**: `matugen`, `wallust`, `pywal`
- **Execution**: Non-interactive command invocation with silent stdio redirection (`Stdio::null()`) to prevent terminal clutter while keeping theme color generation fully automated.
- **Reload Hooks**: Executes application reload signals (`pkill -SIGUSR1 kitty`, `waybar`, etc.) after color scheme updates.

---

## Crate Layout (`wallr-core`)

- `src/daemon/`: Wayland event loop, layer-shell surface lifecycle, and IPC server.
- `src/renderer/`: `wgpu` device/queue setup, pipeline initialization, texture creation, and frame presentation.
- `src/animated/`: Animated GIF decoding with raw or zstd-compressed frame caches and wall-clock playback pacing.
- `src/video/`: FFmpeg decoding, hardware acceleration selection, and PTS-based playback scheduling.
- `src/shader/`: WGSL shader descriptors and module loading.
- `src/wallpaper/`: Engine orchestration, diagnostics (`doctor`), and validation.
- `src/theme/`: Theme provider dispatchers (`matugen`, `wallust`, `pywal`) and hook execution.
- `src/animation/`: YAML spec parsing, effect uniform computation, and timeline evaluation.
- `src/packages/`: Package registry, local resolution, and remote fetching.
- `src/config/`: Configuration parsing, merging, path expansion, and size/duration helpers.
- `src/cache/`: SHA-256 texture caching and persistent state serialization.
- `src/ipc/`: Async IPC client and server transport primitives.
