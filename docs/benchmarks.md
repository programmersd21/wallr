# Benchmarks

Wallr has two benchmark paths:

```sh
./scripts/benchmark-wallpapers.sh
./scripts/benchmark-video.sh assets/demo.mkv
```

The wallpaper benchmark compares switching requests with awww in a live
Wayland session. It measures daemon memory, CPU snapshots, and command
latency. The video benchmark measures decoder throughput and reports whether
the requested backend produced hardware frames.

The video benchmark drives `wallr-core`'s `video_probe` example, which decodes
a file for a few seconds and prints the backend and decoder state it actually
achieved:

```sh
cargo run --release -p wallr-core --example video_probe -- assets/demo.mkv nvdec
```

`video_probe` exits non-zero when the decoder fails or the backend falls back,
so it can gate a script or CI step on its exit status rather than parsing
`result:`. It needs no Wayland session and no GPU.

Results are machine-specific. They must include the compositor, GPU, driver,
display configuration, input media, and Wallr version before being compared.
Decoder throughput is not a substitute for end-to-end presentation or
multi-monitor frame-time measurements.
