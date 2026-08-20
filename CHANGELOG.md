# Changelog

## 0.3.2

- **Fix `libavutil.so.58: cannot open shared object file` on upgraded systems**: previously the Linux release binary dynamically linked the system FFmpeg libraries, so it broke with `error while loading shared libraries` whenever the system FFmpeg ABI moved on (e.g. `libavutil.so.58` → `libavutil.so.61` after an upgrade). Release binaries are now built with **statically linked FFmpeg** via the new `static-ffmpeg` cargo feature (FFmpeg built statically through vcpkg in CI), making the artifact self-contained and immune to future system FFmpeg upgrades. Source builds (`cargo install`) still link against the system FFmpeg and require a latest-but-matching FFmpeg.

## 0.3.1

- **Recovering playback after surface stalls logic**: merged in PR #15 by @Luquatic

- **Moved NV12 conversion to GPU**: merged in PR #14 by @Luquatic

- **Fix publish-blocking workspace dependencies**: commit 96cbfb3 accidentally dropped `notify`, `dirs`, `glob`, `which`, `sha2`, `hex`, `humansize`, `ffmpeg-next` and `crossbeam-channel` from `[workspace.dependencies]` while `wallr-core` and `wallr` still inherited them via `.workspace = true`, breaking `cargo publish`. The dependencies are restored.

- **Fix dead FPS pacer in video playback**: the `--max-fps` pacing loop in the video path declared `last_present` without a type annotation and never assigned it, so the limit silently never activated and the crate failed to compile in a fresh package build. The variable is now typed and stamped after each presented frame.

- **Prevent oversized wallpaper restart loops**: static images are reduced to render-appropriate output dimensions before GPU upload, texture allocations are validated against the adapter limit, and wallpaper state is persisted only after a successful commit. Startup restoration can fall back to the previous valid wallpaper instead of repeatedly aborting on poisoned state.

## 0.3.0

- **Stabilize explicit-sync and NVDEC lifecycles** — merged in PR #13 from @Luquatic.

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
