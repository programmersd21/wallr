# wallr-core

Core engine for `wallr`: daemon, renderer, animation, video, and theme integration.

## modules

- `daemon`: wayland layer-shell surfaces, IPC server, render state
- `renderer`: wgpu pipelines, textures, frame presentation
- `animated`: GIF decoding and playback
- `video`: FFmpeg decoding, hardware acceleration, scheduling
- `animation`: transition effects and uniform computation
- `theme`: provider dispatch and hooks
- `ipc`: async Unix socket transport
- `config`: configuration parsing and validation

## usage

```toml
[dependencies]
wallr-core = "0.8.0"
```

## features

- `static-ffmpeg`: statically link FFmpeg

## license

MIT. See [LICENSE](../LICENSE).
```
