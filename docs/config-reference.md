# Configuration reference

Configuration is YAML, read from `~/.config/wallr/config.yaml` with command-line overrides on top.

```yaml
wallpaper: {default: ~/Pictures/Wallpapers, mode: fill, monitors: [], loop_video: true, mute: true}
theme: {provider: matugen}
matugen: {enabled: true, mode: dark, scheme: scheme-tonal-spot, contrast: 0, wait: true, args: []}
video: {hw_decode: auto, preferred_gpu: auto, preload_frames: 2}
hooks: {before: [], after: [], error: []}
reload: []
daemon: {auto_start: true, socket: "$XDG_RUNTIME_DIR/wallr.sock", max_fps: 60}
```

`wallpaper.mode` accepts: `fill` (cover, default), `fit` (contain with letterbox), `stretch`, `center` (1:1), or `tile` (repeat).

`video.preload_frames` is clamped to 1..=3 live (values above 64 are rejected on reload); `daemon.max_fps` accepts 1..=240 (`null`/`0` disables the cap). `wallr reload` re-reads the config from disk, validates it, and live-applies `hw_decode`/`preload_frames`/`max_fps` without rebuilding GPU state; an invalid config is rejected and the previous valid configuration is kept.
