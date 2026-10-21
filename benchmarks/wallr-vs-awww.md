# Wallr vs awww benchmark

> Local release-build comparison of wallpaper switching performance.
> Latency is command submission time, not measured presentation time.
> Lower latency, RAM, and CPU values are better.

- Date (UTC): 2026-10-21 13:25:39
- Host: nyx
- Wallr client binary: `/home/programmersd/Projects/github/wallr/target/release/wallr`
- Benchmark IPC socket: `/run/user/1000/wallr-benchmark-20261021-132539.sock`
- awww namespace: `wallr-benchmark-20261021-132539`
- Wallr version: wallr 0.8.3
- awww version: awww 0.12.1
- Wayland display: wayland-1
- Wallr daemon FFmpeg linkage: dynamic (libavcodec.so.63, libavformat.so.63, libavutil.so.61, libswscale.so.10)
- Memory baseline: sampled after both daemons showed the same image for 5 seconds


## How to read this report

- **Latency:** time required to submit a wallpaper change.
- **RAM:** resident memory used by the daemon, shown in MiB.
- **CPU:** average process CPU use over a one-second sample.
- **Relative:** how much slower the second command was than Wallr.
- **Verdict:** a winner is reported only for the workload actually measured.
- **Winner:** every step ends with a 🏆 winner line so you can skim the report.

- Wallr daemon binary: `/home/programmersd/Projects/github/wallr/target/release/wallr-daemon`
- Wallr daemon target: `/home/programmersd/Projects/github/wallr/target/release/wallr-daemon`
## 📊 Daemon baseline

| Program | RAM (MiB) | CPU | Threads |
|:--|--:|--:|--:|
| Wallr | 29.5 | 0.0% | 3 |
| awww | 2.7 | 0.0% | 1 |

| Program | RSS (MiB) | PSS (MiB) | Anonymous (MiB) | Private (MiB) | Non-anonymous RSS (MiB) |
|:--|--:|--:|--:|--:|--:|
| Wallr | 29.5 | 22.2 | 5.4 | 20.9 | 24.1 |
| awww | 2.7 | 0.6 | 0.2 | 0.6 | 2.5 |

> 🏆 Winner (idle baseline RAM): awww: 11.12× leaner than Wallr (2.7 MiB vs 29.5 MiB).
> ℹ️ CPU sample (idle baseline): Wallr 0.0% vs awww 0.0% (one-second process CPU time).

## ⚡ Hot same-image static latency

This measures an idempotent request after both daemons already show the image.
🏆 The fastest mean latency wins this round.

| Command | Mean Wall Time [ms] | Change | Factor |
|:---|---:|---:|:---|
| `wallr set '/home/programmersd/Projects/github/wallr/samples/image_04.png' --duration 0 --no-theme` | 2.8 ± 2.8 |  |  |
| `awww img --namespace 'wallr-benchmark-20261021-132539' '/home/programmersd/Projects/github/wallr/samples/image_04.png'` | 32.1 ± 5.1 | +1029.3% | (11.3x slower) |

> 🏆 Winner: Wallr: 11.46× faster than awww (2.8 ms vs 32.1 ms).

## 🚀 Zero-duration static path

Each timed command switches from a different static image with transitions disabled.
🏆 The fastest mean latency wins this round.
Wallr uses `wl_shm` when the output has not used the GPU presentation path; otherwise it preserves the GPU surface protocol for compositor compatibility.

| Command | Mean Wall Time [ms] | Change | Factor |
|:---|---:|---:|:---|
| `wallr set '/home/programmersd/Projects/github/wallr/samples/image_04.png' --duration 0 --no-theme` | 15.6 ± 3.1 |  |  |
| `awww img --namespace 'wallr-benchmark-20261021-132539' '/home/programmersd/Projects/github/wallr/samples/image_04.png'` | 22.9 ± 3.2 | +46.4% | (1.5x slower) |

> 🏆 Winner: Wallr: 1.47× faster than awww (15.6 ms vs 22.9 ms).

## 🔄 Distinct-image switch latency

This measures changing to several different images.
🏆 Each image below is its own round with its own winner.

### 🖼️ image_04.png
| Command | Mean Wall Time [ms] | Change | Factor |
|:---|---:|---:|:---|
| `wallr set '/home/programmersd/Projects/github/wallr/samples/image_04.png' --duration 0 --no-theme` | 17.1 ± 2.1 |  |  |
| `awww img --namespace 'wallr-benchmark-20261021-132539' '/home/programmersd/Projects/github/wallr/samples/image_04.png'` | 24.4 ± 5.0 | +42.3% | (1.4x slower) |

> 🏆 Winner: Wallr: 1.43× faster than awww (17.1 ms vs 24.4 ms).
### 🖼️ image_05.png
| Command | Mean Wall Time [ms] | Change | Factor |
|:---|---:|---:|:---|
| `wallr set '/home/programmersd/Projects/github/wallr/samples/image_05.png' --duration 0 --no-theme` | 17.3 ± 3.1 |  |  |
| `awww img --namespace 'wallr-benchmark-20261021-132539' '/home/programmersd/Projects/github/wallr/samples/image_05.png'` | 26.3 ± 4.6 | +51.7% | (1.5x slower) |

> 🏆 Winner: Wallr: 1.52× faster than awww (17.3 ms vs 26.3 ms).
### 🖼️ image_06.png
| Command | Mean Wall Time [ms] | Change | Factor |
|:---|---:|---:|:---|
| `wallr set '/home/programmersd/Projects/github/wallr/samples/image_06.png' --duration 0 --no-theme` | 17.7 ± 3.3 |  |  |
| `awww img --namespace 'wallr-benchmark-20261021-132539' '/home/programmersd/Projects/github/wallr/samples/image_06.png'` | 25.1 ± 4.2 | +41.5% | (1.4x slower) |

> 🏆 Winner: Wallr: 1.42× faster than awww (17.7 ms vs 25.1 ms).

## 🧘 Post-workload resident state

This shows memory and CPU after all wallpaper changes finish.

| Program | RAM (MiB) | CPU | Threads |
|:--|--:|--:|--:|
| Wallr | 21.3 | 0.0% | 3 |
| awww | 2.6 | 0.0% | 1 |

| Program | RSS (MiB) | PSS (MiB) | Anonymous (MiB) | Private (MiB) | Non-anonymous RSS (MiB) |
|:--|--:|--:|--:|--:|--:|
| Wallr | 21.3 | 14.0 | 0.6 | 12.7 | 20.7 |
| awww | 2.6 | 0.5 | 0.1 | 0.5 | 2.5 |

RSS change from the settled baseline after the switching workload:

| Program | RSS change |
|:--|--:|
| Wallr | -8.2 MiB |
| awww | -0.1 MiB |

> 🏆 Winner (post-workload RAM): awww: 8.30× leaner than Wallr (2.6 MiB vs 21.3 MiB).
> ℹ️ CPU sample (post-workload): Wallr 0.0% vs awww 0.0% (one-second process CPU time).

## 🩺 Daemon health

| Check | Result |
|:--|:--|
| Wallr release daemon after workload | running |

## 🏁 Measured result

> 🏆 Overall winner: Wallr: 5 round(s) won vs 2 for awww (0 tied).

Wallr and awww are optimized for different trade-offs. This run measures
Wallr's switching latency against awww's switching latency. It does not
establish a universal winner across all GPUs, compositors, resolutions,
idle states, animated images, or video formats.

Use the switching tables for latency decisions and the resident-state
table for idle RAM/CPU decisions.

Measured scope: this run reports the measured winner for each workload.
Static switching, idle resource use, and duplicate requests are not
universal claims; repeat the benchmark on the target compositor and GPU.

Report saved to: `/home/programmersd/Projects/github/wallr/benchmarks/wallr-vs-awww.md`
