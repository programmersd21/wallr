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

Results are machine-specific. They must include the compositor, GPU, driver,
display configuration, input media, and Wallr version before being compared.
Decoder throughput is not a substitute for end-to-end presentation or
multi-monitor frame-time measurements.
