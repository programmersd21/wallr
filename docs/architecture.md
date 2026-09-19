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

### 0. Performance model

Wallr separates event-driven state changes from frame-driven work. A static
wallpaper is uploaded once and has no render loop; only a transition, animated
image, or video owns the render lock and presents frames. Repeating the active
request is a no-op, and every detached transition/playback task carries a
generation number so obsolete work stops before doing more GPU work. The
pacer uses timed condition-variable waits and is notified by state changes,
which keeps pause, resume, and wallpaper replacement responsive without a
polling wakeup.

Zero-duration static sets can use a compositor-owned `wl_shm` buffer. Wallr uses
the same output-aware image sizing as the GPU loader, copies the resulting
pixels into an SCTK slot, commits it, then drops the current GPU image. The
slot pool tracks compositor release events before reuse. This path is limited
to full-output static images; transitions, pixel-sensitive modes, GIF/APNG,
and video intentionally remain on wgpu where their rendering requirements
cannot be represented by a single shared-memory buffer.

Static pixel sizing and RGBA conversion are renderer-independent. This keeps
the `wl_shm` path free of a direct wgpu-device dependency and enables lazy GPU
initialization without changing image semantics. When resizing is required,
the helper uses `fast_image_resize`'s CPU-accelerated Lanczos3 implementation;
same-sized images avoid the resize entirely.

Per-output dynamic resources are grouped in `GpuState`, separate from static
Wayland and shared-memory state. The daemon creates layer surfaces and small
shared-memory pools at startup, but leaves adapter/device discovery, pipelines,
swapchain surfaces, and per-output uniform buffers uninitialized. The first
transition, animated image, or video request calls `ensure_gpu`; the renderer
is then initialized once and shared across outputs.

Cached static wallpapers use this zero-duration path during daemon restore, so
startup does not perform a synthetic transition or allocate a temporary GPU
image. Animated and video wallpapers continue through their dynamic pipeline.

The binary uses Tokio's current-thread scheduler for its control plane. Image
decoding, GPU submission, video decoding, and transition presentation are
explicitly dispatched to blocking or dedicated playback tasks, so an idle
daemon does not retain a pool of sleeping executor workers. This is especially
useful on laptops and small systems; it does not serialize the actual render
work. The blocking pool is capped at four workers, preventing a burst of CLI
requests from permanently inflating the daemon's thread count.

Animated GIF frames use a bounded 32 MiB cache. Small animations stay cached
for smooth loop playback, while larger animations fall back to bounded
compressed/on-demand decoding instead of allowing one wallpaper to dominate
daemon memory. GIF bytes are snapshotted to RAM once per set, so loop wraps
re-decode from memory without re-reading the file.

An output surface does not switch back to `wl_shm` after it has presented
through wgpu. This avoids mixing explicit-sync GPU commits with bufferless
shared-memory commits, which some compositors reject when no acquire timeline
is attached. Static-only outputs still use the low-memory `wl_shm` path.

Static wallpapers ping-pong between two shared-memory buffers: while one is
displayed (compositor-held), the idle sibling is guaranteed released, so
steady-state switching never allocates. If both slots are busy under flood,
the set falls back to the GPU path instead of growing the pool, because pool
growth is process-lifetime high-water billed to resident memory.

Once an output goes quiet, a reaper task returns its pool to the OS within
about a second: no set in flight plus no live transition, video, or GIF
(render lock free) means the compositor already owns the displayed pixels,
so both buffers are dropped and the pool is replaced with a fresh minimal
one. The next static set recreates what it needs on demand.

The daemon configures FIFO surfaces with a one-frame maximum latency target,
uses shared immutable samplers and cached pipelines, requests the low-power
adapter preference, and asks wgpu to favor memory usage. These are policies,
not guarantees: the compositor and backend remain free to choose their own
implementation. On Linux, Wallr limits wgpu instance probing to Vulkan and
OpenGL because those are the supported native Wayland rendering backends;
other platform backends are not initialized.

For reproducible live measurements against awww, run
`scripts/benchmark-wallpapers.sh` from a Wayland session. It measures daemon
RSS/CPU, repeated same-image requests, distinct-image switches, static
`wl_shm` commits, GIF submission latency when `samples/extra.gif` exists, and
post-test RSS, PSS, anonymous memory, private memory, and non-anonymous RSS.
Static comparisons disable transitions for both clients so Wallr is not
penalized for rendering work that awww was not asked to perform.
The additional memory columns help distinguish Wallr's wgpu/driver residency
from image-buffer memory. Results are workload-specific; benchmark output must
be retained with the compositor, GPU, resolution, and versions used. The
harness settles both daemons on the same initial image before baseline sampling,
so a cached Wallr restore is not compared with an empty awww daemon.

### 1. Native Wayland Layer Shell (`smithay-client-toolkit`)
- **Protocol**: `wlr-layer-shell-unstable-v1`
- **Layer**: `Layer::Background`
- **Behavior**: Binds to Wayland display server, registers a full-screen background surface per output, handles `scale_factor_changed` and `configure` events dynamically, and sets `wl_surface.set_buffer_scale(scale_factor)` for crisp 1:1 physical pixel rendering on 4K and HiDPI displays.
- **Per-monitor**: Each Wayland output gets its own `LayerSurface`, wgpu `Surface`, and `RenderState`, stored in a `HashMap<String, Arc<Mutex<RenderState>>>` keyed by output name (e.g. `DP-1`, `HDMI-A-1`). Output names are resolved via `wl_output` v4; compositors that don't provide names fall back to make/model or a generated ID. The daemon performs 5 roundtrips at startup to ensure all outputs are discovered, even on compositors that deliver output events lazily.

### 2. GPU Rendering Pipeline (`wgpu`)

The daemon keeps a shared renderer manager, initially empty. Static-only
operation remains entirely on the compositor-owned `wl_shm` path. On demand,
`GpuState` creates the wgpu surface for the affected output and retains the
dynamic resources needed by transitions and live media. This keeps startup and
idle RSS low while preserving the existing GPU path for workloads that need it.
- **Backend**: Vulkan / OpenGL / Metal (via `wgpu` abstraction)
- **Shader Pipeline**: Single-pass WGSL shader (`effects.wgsl`)
- **Uniform Buffer**: Tracks separate old/new image aspect ratios, screen resolution, animation progress (`0.0..1.0`), active effect type index (`0` fade, `1` wipe, `2` slide, `3` wave, `4` grow, `5` outer), effect parameters (`param_a` to `param_d`), effect origin (`origin`), travel direction (`direction`), easing mode, and scaling mode (`scaling_mode`: `0` fill, `1` fit, `2` stretch, `3` center, `4` tile). Struct is 80 bytes (`Vec2`-aligned, size padded for WGSL uniform layout).
- **Aspect Correction**: Static images larger than their useful render size are reduced once before upload while preserving enough pixels for the selected output and scaling mode. The fragment shader still owns the final aspect-ratio mapping: `fill` covers and crops, `fit` contains with letterboxing/pillarboxing, `stretch` ignores aspect ratio, `center` remains 1:1, and `tile` repeats. Center/tile sources retain native dimensions and return a clear error when they exceed the adapter limit rather than silently changing their pixel-sensitive scale. Circular effects (`grow`, `outer`) additionally convert UVs into pixel-aspect-corrected space before taking `distance()`, so expanding rings are true circles on any monitor, never ovals.
- **Stable Image Registration**: The old and new source textures each keep their own immutable `fill` crop for the whole transition. Effects animate blend values and reveal masks in screen space; they do not translate or rescale the wallpaper texture. This prevents the visible “jump” that occurs when images with different source dimensions are changed mid-transition.
- **Smoothness**: Every transition is eased with a configurable curve (`linear` / `ease_in` / `ease_out` / `ease_in_out` / `bezier`, default awww-style cubic bezier) and rendered one frame per vsync. The daemon presents with `PresentMode::Fifo`, so `get_current_texture()` blocks until the previous frame is displayed, pacing animations to the monitor refresh rate. Progress is derived from wall-clock time rather than a frame counter, so a transition lasts exactly its configured `duration` on any refresh rate (frame-count pacing would run too fast on high-refresh panels and too slow on low ones). The ease-in-out tail keeps visible motion almost to the last frame, so a crossfade never appears to stagger to a halt before the transition finishes.
- **Non-blocking transitions**: The daemon commits the new wallpaper state immediately and renders the visual transition on a detached background task, serialized by a render lock. `wallr set` returns as soon as the image is committed, never waiting on GPU presents; theme/hooks/reload run on a further detached task guarded by a generation counter, so rapid switches supersede obsolete theme work. Repeating the active path/mode/effect returns before decode or allocation. The render-state map lock is held only to snapshot targets, never across decode/upload/theme work, so hotplug and concurrent IPC stay responsive. If the compositor stops presenting (monitor off, suspend), the render task parks inside the present without freezing the IPC loop, and later transitions simply queue behind it. A stale swapchain (`Outdated`/`Lost`) is reconfigured in place and the transition continues instead of blanking. Obsolete transition resources are explicitly dropped and polled back to the backend after cancellation.
- **Live wallpapers**: when the committed file is an animated GIF, `AnimatedImage` scans timing metadata first and decodes frames on demand from an in-memory snapshot of the file. If the raw RGBA total fits the 32 MiB budget, frames are cached as-is and playback is a memcpy per frame; larger animations are stored as zstd-compressed streams and decompressed through a persistent `zstd::bulk::Decompressor` context. The first frame becomes the transition's incoming texture, and when the transition ends the render task switches to playback: it computes the absolute wall-clock boundary of the next frame (`frame_start(index+1)` plus whole-loop offsets, so pacing survives animation wrap-around) and presents at that deadline, uploading the next frame into an idle double-buffered texture during the sleep via a mapped staging buffer (copied with `copy_buffer_to_texture`; unaligned widths fall back to `update_texture`). A playback generation counter is bumped on every commit, so a queued playback loop stops itself the moment a newer wallpaper supersedes it. GIF playback respects `wallr ipc pause/resume` commands and preserves timeline position when paused by tracking accumulated pause time. Static images skip playback entirely and present a single frame.
- **Video playback**: `video::VideoPlayback` decodes MP4/WebM/MKV with FFmpeg. Hardware acceleration can be set to `auto` (tries all backends in priority order: NVDEC, VAAPI, VideoToolbox), a specific backend (tries that backend then falls back to software), or `software` (software-only, no hardware attempts). The active decoder backend is reported immediately after successful initialization. Frames are delivered on PTS timing through a small bounded queue and uploaded with the same texture pipeline; `wallpaper.loop_video` restarts the stream at EOF for seamless looping. `wallr ipc pause/resume/seek/info` control playback, and the video path is disabled for static images.
- **Previous-frame compositing**: The daemon keeps the previous decoded wallpaper texture as the outgoing source and reveals the new texture over it. Per-output state is atomically persisted only after the replacement commits, rotating the former path into a previous-wallpaper slot. Startup retries explicitly transient failures and falls back to that previous valid path when restoration still fails.

### 2b. Effect to Uniforms Path
Every transition (`wallr set --effect ...`) resolves to a single `animation::Effect` value, which `compute_effect_uniforms(effect, progress)` converts into an `EffectUniforms` struct (effect type, eased progress, `param_a` to `param_d`, origin, direction, easing mode). The daemon feeds this into `Renderer::render_frame`, so CLI flags and IPC payloads share one identical code path:

```
CLI --effect/--origin/--angle/--easing/...   ┐
IPC Preview { effect, ... }                  ├─► Effect ─► compute_effect_uniforms ─► EffectUniforms ─► WGSL
```

### 3. Daemon & Unix IPC Server (`tokio`)
- **Socket**: `$XDG_RUNTIME_DIR/wallr.sock` (configurable)
- **Protocol**: JSON Line protocol over `tokio::net::UnixListener`, capped at 64 KiB per request with path/monitor/duration/effect validation before any decode or GPU work; malformed commands receive an error response instead of a dropped connection
- **State Machine**: Holds persistent layer-shell surface handle and previous/current texture memory. Accepts IPC commands (`Pause`, `Resume`, `Reload`, `Preview`, `Stop`, `Status`, `Info`, `Seek`, `Blank`, `Restore`) without recreating Wayland windows. The `Preview` command carries a full serialized `Effect` (name + all parameters), so every effect/position/easing combination is expressible over the wire. `Info` reports `not initialized` for outputs that never needed the GPU instead of failing. `Stop` removes the socket file before the process exits, so a stale socket never survives a clean shutdown. `Blank` displays black without replacing the persisted wallpaper; `Restore` validates paths and returns to the previous wallpaper.
- **Crate Layout**: `wallr-common` also provides `effect_from_name`, `effect_names`, and `apply_effect_overrides`, the shared helpers the CLI uses to translate `--effect/--origin/--angle/--easing/--direction/--from/--to/...` flags into an `Effect`.

### 4. Persistent State

The daemon remembers the last wallpaper per output so it can restore it on restart and after hotplug. State is kept under the cache directory (usually `~/.cache/wallr`, or `$XDG_CACHE_HOME/wallr`):

- `~/.cache/wallr/last_wallpaper/<OUTPUT>`: path of the last wallpaper successfully applied to that output (for example `DP-1`, `HDMI-A-1`). Check it with `cat ~/.cache/wallr/last_wallpaper/DP-1` or `ls ~/.cache/wallr/last_wallpaper/`.
- `~/.cache/wallr/previous_wallpaper/<OUTPUT>`: the previous wallpaper for that output, kept so a failed restore can fall back to it.
- `~/.cache/wallr/theme/<hash>.png`: first-frame stills extracted from video or GIF wallpapers for theme providers (Matugen, Wallust, Pywal). Cached by source path plus mtime.

The daemon writes `last_wallpaper` only after a wallpaper commit succeeds. On startup it tries `last_wallpaper` and falls back to `previous_wallpaper` if that file is missing. `wallr ipc blank` does not overwrite this state, and `wallr ipc restore` reads from it.

### 5. Color Theme Pipeline (`theme`)
- **Providers**: `matugen`, `wallust`, `pywal`
- **Execution**: Non-interactive command invocation with silent stdio redirection (`Stdio::null()`) to prevent terminal clutter while keeping theme color generation fully automated.
- **Reload Hooks**: Executes application reload signals (`pkill -SIGUSR1 kitty`, `waybar`, etc.) after color scheme updates.

---

## Crate Layout (`wallr-core` plus shared protocol)

- `wallr-common/`: effect/IPC/CLI/config vocabulary shared by the client and
  the engine. No GPU, Wayland, or decoding dependencies.
- `src/daemon/`: Wayland event loop, layer-shell surface lifecycle, and IPC server.
- `src/renderer/`: `wgpu` device/queue setup, pipeline initialization, texture creation, and frame presentation.
- `src/animated/`: Animated GIF decoding with raw or zstd-compressed frame caches and wall-clock playback pacing.
- `src/video/`: FFmpeg decoding, hardware acceleration selection, and PTS-based playback scheduling.
- `src/shader/`: WGSL shader descriptors and module loading.
- `src/wallpaper/`: Engine orchestration and diagnostics (`doctor`).
- `src/theme/`: Theme provider dispatchers (`matugen`, `wallust`, `pywal`) and hook execution.
- `src/animation/`: Built-in transition effects, uniform computation, and CLI flag mapping.
- `src/config/`: Configuration parsing, merging, path expansion, and duration helpers.
- `src/ipc/`: Async IPC client and server transport primitives.
