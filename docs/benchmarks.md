# benchmarks

The scripts cover separate workloads:

```sh
./scripts/benchmark-wallpapers.sh [image ...]
./scripts/benchmark-video.sh [video]
```

Both benchmarks use dynamic FFmpeg by default. To require static linkage, set
`BENCHMARK_FFMPEG_LINKAGE=static`; prepare the toolchain with
`scripts/install-static-ffmpeg.sh` before building. Each script verifies the
selected linkage and records it in the report. The wallpaper script also needs
a live wayland session, `hyperfine`, and awww. It starts isolated Wallr and
awww daemons with temporary state. Its switching rounds reset both daemons to
another image before each timed request, so they measure a real switch rather
than repeated no-op requests. The display is given
750 ms to settle before timing each switch, avoiding a benchmark-only burst of
outstanding compositor buffers. The initial memory baseline is sampled after
both daemons show the same image for five seconds. It reports command
submission latency, daemon memory, and one-second process cpu samples. The
report includes the RSS change from the settled baseline to show memory
retained by the workload. Submission latency does not measure when the new
image becomes visible. Set
`WALLR_BIN` and `WALLR_DAEMON_BIN` to choose the binaries; use
`BENCHMARK_REPORT` to choose the report path. The script creates temporary
wallpaper layers during the run and removes its daemons afterward. Systems
without FFmpeg development libraries can use the static helper and point both
override variables at the same verified static release build. The default
report path is `benchmarks/wallr-vs-awww.md`, so each run replaces the previous
report; set `BENCHMARK_REPORT` only when a separate report is needed.

Dynamic builds use the default Cargo features. Static builds add
`--features static-ffmpeg`. The linkage variable selects and verifies the video
probe build; for wallpaper benchmarks it verifies the already-built daemon.
Official release binaries remain statically linked.

The video script runs `video_probe` with software, VAAPI, and NVDEC. It measures
decoding, not compositor presentation, gpu rendering, or multi-monitor
playback. Hardware results require a matching device and driver.

Keep results with the host, compositor, gpu, driver, display layout, media,
and software versions. Measurements from different machines are not directly
comparable. One run does not establish a universal winner.
