# Changelog

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
