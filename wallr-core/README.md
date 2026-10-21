# wallr-core

wallr's wayland daemon and rendering engine. It contains output management,
wgpu rendering, animated image and video playback, theme providers, and ipc.

```toml
[dependencies]
wallr-core = "0.8.3"
```

The optional `static-ffmpeg` feature enables static ffmpeg linking when the
required build libraries are available. In the repository, run
`scripts/install-static-ffmpeg.sh` to prepare the pinned Linux toolchain.

- `static-ffmpeg`: statically link FFmpeg

## license

MIT. See [LICENSE](../LICENSE).
