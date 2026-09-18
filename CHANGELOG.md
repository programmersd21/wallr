# Changelog

All notable changes to Wallr are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and versions follow [Semantic Versioning](https://semver.org/).

## [0.6.0] - 2026-09-18

### Removed

- Removed the animation package system: YAML specs, timelines, variables,
  `extends`, the local registry, and remote install/fetch (`wallr install`,
  `wallr search`, `wallr new`).
- Removed custom WGSL effects (`custom_effects` transpiler, `shader` effect).
- Removed the preview window (`wallr preview`) and the `winit` dependency.
- Removed directory watching (`wallr watch`) and the `notify` dependency.
- Removed the disk image cache (`wallr cache`) and the `humansize` dependency.
- Removed the `wallr validate` command.
- Reduced transitions to six built-in effects: `fade`, `wipe`, `slide`,
  `wave`, `grow`, `outer` (plus `simple`, directional, `center`, `any`,
  and `random` aliases). Removed `blur`, `zoom`, `pixelate`, `ripple`,
  `dissolve`, and `shader`, with their CLI flags (`--speed`, `--scale`).
  Reveal edges are pixel-scale sharp; the default with no `--effect` flag
  is now a plain linear crossfade, matching the feel of awww's default
  `simple` transition.
- Removed the `animation`, `watch`, `cache`, and `plugins` config sections.
  Existing config files still load; unknown keys are ignored.
- Workspace now ships a `wallr-common` protocol crate (effects, IPC
  vocabulary, CLI shapes, config schema) shared by the client and the
  engine, mirroring the `client`/`common`/`daemon` split. Single `wallr`
  binary and IPC wire format unchanged.
- `wallr set -` reads image bytes from stdin, staged to a temp file.
- Moved `naga` to dev-dependencies (WGSL validation in tests only).

### Changed

- GIF decoding now snapshots the file to memory once per set; loop wraps
  re-decode from RAM instead of re-opening the file.
- Static wallpapers ping-pong between two shared-memory buffers, so
  steady-state switching never allocates; flood pressure falls back to the
  GPU path instead of growing the pool without bound.
- Settled outputs return their shared-memory pools to the OS within about
  a second (the compositor already owns the displayed pixels), so idle
  memory drops back after each static wallpaper. Quiescent heaps are also
  trimmed back at the same time.
- Fixed `wallr ipc info` killing the daemon when no GPU was initialized;
  it now reports `not initialized` for static-only outputs.
- Removed `panic = "abort"` from the release profile: a wallpaper daemon
  must survive unexpected states instead of aborting the process.
- Repeated sets of the active wallpaper skip state-file persistence
  (previously two writes plus two fsyncs per call).
- The daemon tunes glibc malloc at startup (fixed 128 KiB mmap threshold,
  64 KiB trim threshold) so transient decode buffers return to the OS.
- Video decoder queue and preload clamp tightened to at most 3 frames in
  flight plus one pending frame; stale frames are dropped, not preserved.
- Removed `Clone` from video frame types; frame bytes move from decoder
  through queue to GPU upload.
- Added an offscreen GPU test that renders a real fade transition without
  a compositor and asserts the blended pixels, plus a test locking the
  effect-to-shader numbering.
- Fixed GIF playback stopping permanently on a single slow present; timeouts
  now yield briefly and playback continues.
- VA-API now probes every `/dev/dri/renderD*` node instead of hardcoding
  `renderD128`.
- GPU backend probing tries Vulkan alone before initializing the GL driver
  stack (about 15 ms saved on every cold start); GL-only systems still fall
  back automatically.
- Static GPU uploads share the SIMD resize path with the `wl_shm` fast path.

### Measured

- Dependency closure: 341 to 258 crates. Release binary: 11 MiB to 8.8 MiB.
- Live Wayland matrix (`benchmarks/`, same host, vs awww 0.12.1): Wallr leads
  the latency rounds (static switches ~20 ms, GIF submission ~25 ms).
  Post-GIF anonymous memory returns to ~24 MiB (was ~76 MiB); settled
  static outputs return their shm pools, idling near 30 MiB RSS.
- Idle and post-workload RSS still trail aww's minimal C daemon; process
  PSS and private-dirty memory are sub-MiB at idle. See the benchmark
  reports for full tables and methodology.

## [0.5.0] - 2026-09-16

### Added

- Added repeatable Wayland benchmark coverage against awww.
- Added short CLI flags, including `-e`, `-d`, `-o`, `-a`, `-m`, `-t`, and `-c`.
- Added the zero-duration static-image `wl_shm` fast path.
- Added `WALLR_SOCKET` support for isolated daemon instances.

### Changed

- Made GPU initialization genuinely lazy: static daemon startup now avoids
  adapter/device discovery, shader and pipeline creation, swapchain surfaces,
  and per-output GPU uniform buffers until dynamic content is requested.
- Shared the lazily-created renderer across outputs while retaining separate
  per-output surfaces and uniforms for correctness.
- Verified the local release daemon through static and GIF benchmark workloads;
  static-only RSS dropped substantially after removing eager GPU residency.
- Switched the long-lived daemon/CLI executor to Tokio's current-thread
  scheduler. Blocking decode/render work already uses dedicated blocking tasks,
  so this removes idle worker threads without reducing rendering concurrency.
- Reduced the animated-frame cache ceiling from 256 MiB to 128 MiB. This keeps
  normal 1080p GIF playback cached and smooth while bounding the memory impact
  of unusually large animations.
- Capped Tokio's blocking pool at four workers so bursty IPC benchmarks cannot
  leave an unbounded number of sleeping workers resident.
- Simplified the static `wl_shm` pixel swizzle to direct channel stores,
  removing a per-pixel temporary slice from the upload hot loop.
- Replaced the scalar Lanczos resize used for downscaled images with
  `fast_image_resize`'s CPU-accelerated implementation while preserving the
  same Lanczos3 quality. A measured 3840×2160 static switch stayed around
  20–27 ms against awww's roughly 95–112 ms on the benchmark host.
- Corrected benchmark baseline methodology by applying the same initial static
  image to both daemons before measuring resident resources.

- Prevented explicit-sync protocol failures when a layer surface has already
  presented through wgpu: later static requests remain on that surface's GPU
  path instead of switching it to an incompatible `wl_shm` commit.
- Benchmark reports now stop live animated playback before sampling final idle
  RAM and CPU, separating settled daemon cost from intentional decoder state.
- Added `BENCHMARK_INCLUDE_ANIMATED=0` for repeatable static-only residency
  measurements while evaluating renderer lifetime changes.
- Cached static wallpapers now restore without an unnecessary startup
  transition, avoiding temporary GPU work during daemon initialization.
- Static image switches now skip the GIF scanner entirely for non-GIF files,
  avoiding a redundant full-file read before normal image decoding.
- Moved static `wl_shm` pixel preparation into a renderer-independent helper,
  establishing the boundary needed for lazy GPU initialization.
- Grouped per-output GPU resources behind a dedicated ownership boundary,
  preparing safe optional GPU state without changing dynamic rendering behavior.

- **GPU Animation Overhaul (awww/swww parity & precision)**:
  - Replaced radial wipe fallbacks with true directional linear sweep (`linear_wipe_reveal`) supporting angled sweeps and directional vectors (`-a <DEG>`, `--direction <X,Y>`).
  - Upgraded Gaussian blur from 8-sample radial ring to a 16-sample golden-angle circular bokeh disk, eliminating banding and aliasing.
  - Implemented C2-continuous quintic smootherstep ($6t^5 - 15t^4 + 10t^3$) across CPU evaluation and WGSL shaders, guaranteeing zero velocity and acceleration at start/end frames.
  - Added exact GPU mirrors of back-out overshoot (`emphatic`) and critically-damped harmonic settling (`spring`) without unwanted oscillations.
  - Upgraded texture minification and mipmap filters from `Nearest` to `Linear` on static and video samplers to eliminate sub-pixel shimmer during zoom/scale transitions.
  - Refined all 8 bundled animation packages (`crossfade`, `grow`, `outer`, `wave`, `liquid`, `wipe-blur`, `minimal`, `retro`) with non-overlapping, human-tuned parameters.
- **Clean, Minimal CLI Interface**:
  - Added ergonomic short flags: `-e` (effect), `-d` (duration), `-o` (origin), `-a` (angle), `-m` (monitor), `-t` (theme), `-c` (config).
  - Streamlined `wallr set` / `wallr img` flags and aligned command help strings.
  - Redesigned CLI terminal outputs (`doctor`, `validate`, `cache`, `monitor`, `search`) into clean, minimal, non-bloated tables and indicators.
- Added a zero-duration static-image fast path backed by `wl_shm`: completed
  static images can bypass shader rendering, release their GPU texture, and
  remain compositor-owned until the next change. Transitions, GIFs, and video
  retain the wgpu path.
- Linux renderer initialization now probes only Vulkan and OpenGL, avoiding
  unrelated wgpu backend discovery while preserving a compatibility fallback.
- Fixed static `wl_shm` replacement when the compositor still owns the
  previous buffer; Wallr now allocates a released-aware slot instead of
  silently falling back to GPU textures and accumulating resident memory.
- Static wallpaper requests are idempotent: repeating the active path, mode,
  and effect does not decode, upload, allocate, or start another transition.
- Superseded transitions and live players check their generation before taking
  the render lock and during playback, releasing obsolete GPU resources early.
- Paused and superseded GIF/video pacing now wakes immediately when a new
  command arrives instead of waiting for the previous sleep deadline.
- GIF playback presents at frame boundaries rather than once per display
  refresh, and static content has no animation wakeup path.
- Daemon surfaces use a one-frame FIFO latency target; the renderer requests a
  power-efficient adapter and memory-use-oriented device allocation policy.
- Video decode backpressure is bounded without a tight 1 ms polling loop.
- Added `scripts/benchmark-wallpapers.sh` for repeatable live Wayland
  comparisons against `awww` using hot-path and distinct-image workloads.
- Benchmark reports now include RSS/PSS, anonymous, private, and
  non-anonymous memory categories to diagnose renderer residency instead of
  relying on a single RSS number.
- Fixed startup registration of already-connected outputs so IPC commands use
  the live render states immediately instead of waiting for a hotplug event.
- Added the `WALLR_SOCKET` process override and isolated benchmark daemon
  cleanup, allowing local-release comparisons beside an installed daemon.
- Static wallpaper requests no longer block behind a GPU transition parked in
  a compositor present; they fall back to the nonblocking replacement path so
  IPC remains responsive during suspend, monitor disable, and compositor
  stalls.
- Benchmark idle samples now wait for daemon startup, output discovery, and
  GPU initialization to settle, preventing startup CPU from being reported as
  steady-state idle usage.
- Removed an unused source-image copy from the daemon wallpaper hot path;
  rendering and restore already use the original validated path, while cache
  inspection and clearing remain available explicitly.
- Removed the unused cache manager from daemon-owned `WallpaperEngine` state;
  cache commands now initialize it only when explicitly requested.
- Removed the unused package registry from daemon-owned `WallpaperEngine`
  state; package resolution and installation remain CLI-owned.
- Reduced initial per-output `wl_shm` pool allocation to 4 KiB; Smithay's
  `SlotPool` grows automatically when the first real wallpaper buffer is
  created, avoiding full-resolution shm reservation during daemon startup.

The benchmark harness reports measurements; it intentionally does not claim a
universal ranking because compositor, GPU, image dimensions, and workload all
change the result.

## 0.4.0 - 2026-09-15
Planned Sep 12-14, implemented Sep 15

- **Do less work**: static wallpapers submit once and sleep; transitions run only for their wall-clock duration; video/GIF loops pace to frame boundaries instead of the refresh rate. No render-loop wakeups when nothing changes.
- **GPU resource sharing**: one shared sampler per use (static/video) instead of a new sampler per texture, and a per-format pipeline cache so the first frame per surface format pays for compilation exactly once.
- **IPC hardening**: requests capped at 64 KiB, paths/monitors/durations validated before any decode or GPU work, malformed commands get an error response instead of a dropped connection.
- **Path security**: package names are lexically contained in the registry (`..`, separators, absolute paths rejected); remote `owner/repo` refs restricted to safe characters.
- **Daemon hot path**: the render-state map lock is held only to snapshot targets, never across decode/upload/theme work, so hotplug and concurrent IPC stay responsive. Transitions reconfigure a stale swapchain and continue instead of blanking.
- **Real config reload**: `reload` re-reads config from disk, validates it, keeps the last good config on failure, and live-applies `hw_decode`/`preload_frames`/`max_fps` without rebuilding GPU state.
- **Async theme pipeline**: wallpaper pixels appear first; hooks/theme/reload run detached with a generation counter so rapid A→B→C switches supersede obsolete theme work.
- **Watcher**: reacts to create/modify/close-write (not just create), skips temp/hidden files, debounces per path, ignores partially-written files, and supports video extensions.
- **Bounded video config**: `preload_frames` clamped to 1..=8 and `max_fps` to 1..=240 at creation and reload, keeping decoder queues bounded.

## 0.3.4

- **`wallr install` now actually installs**: `wallr install <username/repo>` (or `github:username/repo`) downloads the package YAML from the repository root on GitHub, validates it, resolves its `extends` chain, and stores it at `~/.local/share/wallr/packages/<repo>/wallr.yaml`. The previous command only re-passed the reference to the local registry resolver and reported success without installing anything.
- **Remote fetch hardened**: packages are fetched from the `main` or `master` branch and validated before use; malformed and path-traversal references are rejected.
- **Reference resolution fixed**: package references are now treated as registry paths instead of being force-split on `/`, which previously loaded the wrong package and silently dropped the second segment.
- **`extends` actually applied**: the base/child merge was implemented and tested but never invoked; package resolution and install now merge parents (base → package → current), with circular references rejected. `github:owner/repo` parents resolve remotely.
- **Removed the placeholder `publish` command**, which printed an acknowledgment without doing anything. The registry surface is now `install`/`search` only.

## 0.3.3

- **Video/GIF wallpapers now theme correctly**: `wallr set` with a video (`mp4`, `webm`, `mkv`, `mov`, `avi`, `m4v`) or GIF previously passed the original file straight to `matugen`/`wallust`/`pywal`, which cannot generate palettes from video. Wallr now extracts the first frame (FFmpeg for video, `image` crate for GIF) to a cached PNG under `~/.cache/wallr/theme/<hash>.png` and passes that to the theme provider. The cache is keyed on source path + mtime and reused until the source changes. All providers benefit with no manual step.
- **Matugen config deprecation warning fixed**: `[config.wallpaper]` with separate `command` + `arguments` now emits `⚠ You should not define arguments inside of [config.wallpaper] anymore. Use the command instead and use the {{ image }} keyword`. Updated docs to use the new single-string form `command = "wallr img --no-theme {{ image }}"` with `{{ image }}` templating and retained the old form as deprecated reference.

## 0.3.2

- **Fix `libavutil.so.58: cannot open shared object file` on upgraded systems**: previously the Linux release binary dynamically linked the system FFmpeg libraries, so it broke with `error while loading shared libraries` whenever the system FFmpeg ABI moved on (e.g. `libavutil.so.58` → `libavutil.so.61` after an upgrade). Release binaries are now built with **statically linked FFmpeg** via the new `static-ffmpeg` cargo feature (FFmpeg built statically through vcpkg in CI), making the artifact self-contained and immune to future system FFmpeg upgrades. Source builds (`cargo install`) still link against the system FFmpeg and require a latest-but-matching FFmpeg.

## 0.3.1

- **Recovering playback after surface stalls logic**: merged in PR #15 by @Luquatic

- **Moved NV12 conversion to GPU**: merged in PR #14 by @Luquatic

- **Fix publish-blocking workspace dependencies**: commit 96cbfb3 accidentally dropped `notify`, `dirs`, `glob`, `which`, `sha2`, `hex`, `humansize`, `ffmpeg-next` and `crossbeam-channel` from `[workspace.dependencies]` while `wallr-core` and `wallr` still inherited them via `.workspace = true`, breaking `cargo publish`. The dependencies are restored.

- **Fix dead FPS pacer in video playback**: the `--max-fps` pacing loop in the video path declared `last_present` without a type annotation and never assigned it, so the limit silently never activated and the crate failed to compile in a fresh package build. The variable is now typed and stamped after each presented frame.

- **Prevent oversized wallpaper restart loops**: static images are reduced to render-appropriate output dimensions before GPU upload, texture allocations are validated against the adapter limit, and wallpaper state is persisted only after a successful commit. Startup restoration can fall back to the previous valid wallpaper instead of repeatedly aborting on poisoned state.

## 0.3.0

- **Stabilize explicit-sync and NVDEC lifecycles** - merged in PR #13 from @Luquatic.

## 0.2.9

- **Cinematic Blank & Restore Transitions**: Added support for custom transition effects and durations to `wallr ipc blank` and `wallr ipc restore` commands via CLI effect flags (e.g. `wallr ipc blank --duration 1.5s --effect blur`).
- **Smooth Desktop Startup Fade-in**: Daemon wallpaper restoration on startup and display hotplug now fades in smoothly over `1s` instead of snapping instantly.
- **Daemon Frame Rate Limit (`--max-fps`)**: Added `--max-fps` CLI flag to `wallr daemon` and `daemon.max_fps` configuration property to tune rendering performance and power consumption.

## 0.2.8

- **Fix surface size crash on fractional scale**: Compositors using non-integer scale factors (e.g. 1.25 on niri, Hyprland) no longer multiply physical dimensions by `scale_factor`, which caused surfaces to exceed GPU texture limits (e.g. 3840x2160 at scale 1.25 was incorrectly rendered as 7680x4320). `mode.dimensions` already returns physical pixels; the redundant multiplication has been removed from all three surface creation paths.

## 0.2.7

- **Fix restore command**: `wallr ipc restore` now properly tracks the last set wallpaper and validates paths before attempting restore. Errors during restore are reported instead of being silently ignored.
- **GIF pause/resume support**: `wallr ipc pause` and `wallr ipc resume` now correctly pause and resume GIF playback in addition to video playback. GIF timeline position is preserved when paused.
- **Fix seek command**: `wallr ipc seek` without `--monitor` now applies to all connected outputs instead of arbitrarily selecting the first output from an unordered map.
- **Hardware acceleration fallback improvements**:
  - New `auto` mode tries all hardware backends (NVDEC, VAAPI, VideoToolbox) in priority order before falling back to software
  - Explicit backend requests (e.g., `nvdec`, `vaapi`) now correctly try the requested backend first, then fall back to software
  - `software` mode now uses software-only decoding without attempting hardware backends first
  - Active decoder backend is reported immediately after successful initialization instead of only when the decoder thread exits
- **Better error reporting**: Restore and seek commands now provide detailed error messages per-output when operations fail.

## 0.2.6

- **#7 Full output hotplug**: Outputs that connect after daemon startup are automatically detected, and a LayerSurface + wgpu surface + render state is created for them. Disconnected outputs are cleaned up (playback stopped, render state removed, surfaces released). Output resolution, scale, and transform changes reconfigure the wgpu surface on the fly. `wallr monitor list` stays synchronized without restarting the daemon. Commands targeting disconnected outputs return an error.

## 0.2.5

- **#9 Fix renderer race condition**: Per-output uniform buffers eliminate cross-output GPU state races. Each output renders independently with its own uniform bind group.
- **#6 Fix monitor targeting**: Unknown `--monitor` names now return an error instead of silently targeting the first output. Without `--monitor`, `set`/`preview` applies to all connected outputs deterministically.
- **#8 Monitor-scoped playback controls**: Added `--monitor` to `pause`, `resume`, `seek`, and `info` IPC commands. Info without `--monitor` reports all outputs.
- **#10 Non-destructive blank/restore**: Added `wallr ipc blank` and `wallr ipc restore` commands. Blank displays black without replacing the persisted wallpaper; restore returns to the previous image. Supports `--monitor` for per-output control.
- **#7 Output hotplug**: Disconnected outputs are cleaned up automatically. Stale render states are removed when Wayland signals output removal.

## 0.2.4

- Fix output name resolution: compositor-provided names (e.g. `DP-1`, `HDMI-A-1`) are now correctly applied instead of always showing `output-{id}`.
- Fallback chain for output names: name -> description -> make+model -> generated ID.
- Increase startup roundtrips from 2 to 5 to catch lazy compositor output events.
- Added tracing for output detection debugging (`wallr -v daemon`).

## 0.2.3

- Per-monitor wallpaper support: each Wayland output gets its own LayerSurface, wgpu Surface, and RenderState. Wallpapers can be set per-output via `--monitor` on CLI.
- `wallr monitor list` and `wallr monitor current` query the daemon via IPC for real output info.
- Per-output last wallpaper persistence: `wallr/last_wallpaper/{output_name}` instead of a single file.
- File watcher applies new images to all connected outputs.
- IPC `Pause`/`Resume` affect all outputs simultaneously.

## 0.2.2

- Shader-level scaling modes: `fill` (cover), `fit` (contain), `stretch`, `center` (1:1), `tile` (repeat). Applied via `--mode` on CLI or `wallpaper.mode` in config.
- Fix NVIDIA hardware acceleration fallback: when both VAAPI and NVDEC are present, the decoder now tries all backends before falling back to software (fixes #4).
- Pass `--mode` and `--monitor` through IPC instead of silently discarding them (fixes #3).
- Remove dead `VideoSource` trait and duplicate impl blocks.
- Remove unnecessary `catch_unwind` in decode thread.
- Remove dead code (`let _ = newest`, `thread_sleep` wrapper).
- Remove restating doc comments across all video module files.
- Clean up AI-generated comment patterns across the codebase.

## 0.2.1

- Pass `--mode` and `--monitor` through IPC instead of silently discarding them (fixes #3).
- Fix NVIDIA hardware acceleration fallback.

## 0.2.0

- Animated GIF playback with zstd-compressed frame cache.
- Video wallpaper support (MP4, WebM, MKV) with hardware-accelerated decoding.
- 11 transition effects with circular reveal system.
- Background daemon with Unix IPC.
- Animation package system with YAML timelines.
- Theme pipeline integration (Matugen, Wallust, Pywal).
- Preview window for testing effects.
- GPU adapter selection for hybrid systems.

## 0.1.0

- Initial release.
