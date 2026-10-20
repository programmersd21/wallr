# Wallr vs awww benchmark

> Production benchmark report for wallpaper switching performance.
> Lower latency, RAM, and CPU values are better.

- Date (UTC): 2026-10-20 08:56:26
- Host: nyx
- Wallr client binary: `/home/programmersd/Projects/github/wallr/target/release/wallr`
- Benchmark IPC socket: `/run/user/1000/wallr-benchmark-20261020-085626.sock`
- Wallr version: wallr 0.8.0
- awww version: awww 0.12.1
- Wayland display: wayland-1


## How to read this report

- **Latency:** time required to submit a wallpaper change.
- **RAM:** resident memory used by the daemon, shown in MiB.
- **CPU:** processor usage sampled by the operating system.
- **Relative:** how much slower the second command was than Wallr.
- **Verdict:** a winner is reported only for the workload actually measured.
- **Winner:** every step ends with a 🏆 winner line so you can skim the report.

- Wallr daemon binary: `/home/programmersd/Projects/github/wallr/target/release/wallr-daemon`
- Wallr daemon target: `/home/programmersd/Projects/github/wallr/target/release/wallr-daemon`
## 📊 Daemon baseline

| Program | RAM (MiB) | CPU | Threads |
|:--|--:|--:|--:|
| Wallr | 29.1 | 5.1% | 3 |
| awww | 2.4 | 0.4% | 1 |

| Program | RSS (MiB) | PSS (MiB) | Anonymous (MiB) | Private (MiB) | Non-anonymous RSS (MiB) |
|:--|--:|--:|--:|--:|--:|
| Wallr | 29.1 | 23.9 | 5.4 | 23.5 | 23.7 |
| awww | 2.4 | 0.4 | 0.1 | 0.3 | 2.3 |

> 🏆 Winner (idle baseline RAM): awww: 12.17× leaner than Wallr (2.4 MiB vs 29.1 MiB).
> ℹ️ CPU snapshot (idle baseline): Wallr 4.4% vs awww 0.4% (instant sample, noisy).

## ⚡ Hot same-image static latency

This measures repeated static requests with transitions disabled.
🏆 The fastest mean latency wins this round.

| Command | Mean Wall Time [ms] | Change | Factor |
|:---|---:|---:|:---|
| `wallr set '/home/programmersd/Projects/github/wallr/samples/image_04.png' --duration 0 --no-theme` | 72.2 ± 81.2 |  |  |
| `awww img '/home/programmersd/Projects/github/wallr/samples/image_04.png'` | 132.2 ± 225.0 | +83.1% | (1.8x slower) |

> 🏆 Winner: Wallr: 1.83× faster than awww (72.2 ms vs 132.2 ms).

## 🚀 Zero-duration static path

This measures a static wallpaper with no transition animation.
🏆 The fastest mean latency wins this round.
Wallr uses `wl_shm` when the output has not used the GPU presentation path; otherwise it preserves the GPU surface protocol for compositor compatibility.

| Command | Mean Wall Time [ms] | Change | Factor |
|:---|---:|---:|:---|
| `wallr set '/home/programmersd/Projects/github/wallr/samples/image_04.png' --duration 0 --no-theme` | 1.4 ± 0.8 |  |  |
| `awww img '/home/programmersd/Projects/github/wallr/samples/image_04.png'` | 26.1 ± 3.8 | +1705.3% | (18.1x slower) |

> 🏆 Winner: Wallr: 18.64× faster than awww (1.4 ms vs 26.1 ms).

## 🔄 Distinct-image switch latency

This measures changing to several different images.
🏆 Each image below is its own round with its own winner.

### 🖼️ image_04.png
| Command | Mean Wall Time [ms] | Change | Factor |
|:---|---:|---:|:---|
| `wallr set '/home/programmersd/Projects/github/wallr/samples/image_04.png' --duration 0 --no-theme` | 2.1 ± 0.9 |  |  |
| `awww img '/home/programmersd/Projects/github/wallr/samples/image_04.png'` | 26.2 ± 3.8 | +1148.8% | (12.5x slower) |

> 🏆 Winner: Wallr: 12.48× faster than awww (2.1 ms vs 26.2 ms).
### 🖼️ image_05.png
| Command | Mean Wall Time [ms] | Change | Factor |
|:---|---:|---:|:---|
| `wallr set '/home/programmersd/Projects/github/wallr/samples/image_05.png' --duration 0 --no-theme` | 1.6 ± 0.4 |  |  |
| `awww img '/home/programmersd/Projects/github/wallr/samples/image_05.png'` | 22.7 ± 1.8 | +1312.1% | (14.1x slower) |

> 🏆 Winner: Wallr: 14.19× faster than awww (1.6 ms vs 22.7 ms).
### 🖼️ image_06.png
| Command | Mean Wall Time [ms] | Change | Factor |
|:---|---:|---:|:---|
| `wallr set '/home/programmersd/Projects/github/wallr/samples/image_06.png' --duration 0 --no-theme` | 4.1 ± 2.3 |  |  |
| `awww img '/home/programmersd/Projects/github/wallr/samples/image_06.png'` | 31.7 ± 5.1 | +682.8% | (7.8x slower) |

> 🏆 Winner: Wallr: 7.73× faster than awww (4.1 ms vs 31.7 ms).

## 🧘 Post-workload resident state

This shows memory and CPU after all wallpaper changes finish.

| Program | RAM (MiB) | CPU | Threads |
|:--|--:|--:|--:|
| Wallr | 50.1 | 4.7% | 6 |
| awww | 2.4 | 0.7% | 1 |

| Program | RSS (MiB) | PSS (MiB) | Anonymous (MiB) | Private (MiB) | Non-anonymous RSS (MiB) |
|:--|--:|--:|--:|--:|--:|
| Wallr | 50.1 | 43.7 | 18.3 | 42.4 | 31.9 |
| awww | 2.4 | 0.4 | 0.1 | 0.3 | 2.3 |

> 🏆 Winner (post-workload RAM): awww: 20.94× leaner than Wallr (2.4 MiB vs 50.1 MiB).
> ℹ️ CPU snapshot (post-workload): Wallr 4.7% vs awww 0.7% (instant sample, noisy).

## 🩺 Daemon health

| Check | Result |
|:--|:--|
| Wallr release daemon after workload | running |

## 🏁 Production verdict

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

Report saved to: `/home/programmersd/Projects/github/wallr/benchmarks/wallr-vs-awww-0.8.0-final.md`
