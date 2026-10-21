# benchmarks

The scripts cover separate workloads:

```sh
./scripts/benchmark-wallpapers.sh [image ...]
./scripts/benchmark-video.sh [video]
```

The wallpaper script needs a live wayland session, `hyperfine`, and awww. It
starts the local release daemon on a separate socket and reports switching
latency, daemon memory, and sampled cpu use. Set `WALLR_BIN` or
`WALLR_DAEMON_BIN` to choose the local binaries. Set `BENCHMARK_REPORT` to
choose the report path. The script changes the displayed wallpaper during the
run and should be used in a test session.

The video script runs `video_probe` with software, VAAPI, and NVDEC. It measures
decoding, not compositor presentation, gpu rendering, or multi-monitor
playback. Hardware results require a matching device and driver.

Keep results with the host, compositor, gpu, driver, display layout, media,
and software versions. Measurements from different machines are not directly
comparable. One run does not establish a universal winner.
