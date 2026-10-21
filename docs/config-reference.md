# configuration

wallr reads `~/.config/wallr/config.yaml` by default. Pass `--config <path>`
to use another file. Missing fields use these defaults:

```yaml
wallpaper:
  default: null
  mode: fill
  monitors: []
  loop_video: true
  mute: true

theme:
  provider: matugen

matugen:
  enabled: true
  mode: dark
  scheme: scheme-tonal-spot
  contrast: 0
  wait: true
  args: []

hooks:
  before: []
  after: []
  error: []

reload: []

daemon:
  auto_start: true
  socket: "$XDG_RUNTIME_DIR/wallr.sock"
  max_fps: 60

video:
  hw_decode: auto
  preferred_gpu: auto
  preload_frames: 2
```

`wallpaper.mode` accepts `fill`, `fit`, `stretch`, `center`, or `tile`.
`wallpaper.monitors` contains entries with a required `name` and optional
`file`. Theme providers are `none`, `matugen`, `wallust`, and `pywal`.

`daemon.max_fps` accepts 1 through 240; `null` or `0` disables the cap.
`video.hw_decode` accepts `auto`, `nvdec`, `vaapi`, or `software`.
`video.preferred_gpu` accepts `auto`, `integrated`, `discrete`, or an adapter
name. `video.preload_frames` is limited to 1 through 3 during playback.

Use `wallr config get <key>` and `wallr config set <key> <value>` to inspect
or change a setting. `wallr reload` validates the file before applying
supported live settings; invalid changes leave the current configuration in
place.
