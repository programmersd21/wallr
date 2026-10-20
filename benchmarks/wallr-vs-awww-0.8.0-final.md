# Wallr vs awww benchmark

> Production benchmark report for wallpaper switching performance.
> Lower latency, RAM, and CPU values are better.

- Date (UTC): 2026-10-20 08:59:41
- Host: nyx
- Wallr client binary: `/home/programmersd/Projects/github/wallr/target/release/wallr`
- Benchmark IPC socket: `/run/user/1000/wallr-benchmark-20261020-085941.sock`
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
| Wallr | 29.2 | 3.7% | 3 |
| awww | 2.4 | 0.4% | 1 |

| Program | RSS (MiB) | PSS (MiB) | Anonymous (MiB) | Private (MiB) | Non-anonymous RSS (MiB) |
|:--|--:|--:|--:|--:|--:|
| Wallr | 29.2 | 22.0 | 5.4 | 20.4 | 23.8 |
| awww | 2.4 | 0.4 | 0.1 | 0.3 | 2.3 |

> 🏆 Winner (idle baseline RAM): awww: 12.17× leaner than Wallr (2.4 MiB vs 29.2 MiB).
> ℹ️ CPU snapshot (idle baseline): Wallr 3.7% vs awww 0.4% (instant sample, noisy).

## ⚡ Hot same-image static latency

This measures repeated static requests with transitions disabled.
🏆 The fastest mean latency wins this round.

| Command | Mean Wall Time [ms] | Change | Factor |
|:---|---:|---:|:---|
| `wallr set '/home/programmersd/Projects/github/wallr/samples/image_04.png' --duration 0 --no-theme` | 1.2 ± 0.4 |  |  |
| `awww img '/home/programmersd/Projects/github/wallr/samples/image_04.png'` | 23.4 ± 2.1 | +1812.1% | (19.1x slower) |

> 🏆 Winner: Wallr: 19.50× faster than awww (1.2 ms vs 23.4 ms).

## 🚀 Zero-duration static path

This measures a static wallpaper with no transition animation.
🏆 The fastest mean latency wins this round.
Wallr uses `wl_shm` when the output has not used the GPU presentation path; otherwise it preserves the GPU surface protocol for compositor compatibility.

| Command | Mean Wall Time [ms] | Change | Factor |
|:---|---:|---:|:---|
| `wallr set '/home/programmersd/Projects/github/wallr/samples/image_04.png' --duration 0 --no-theme` | 1.7 ± 0.7 |  |  |
| `awww img '/home/programmersd/Projects/github/wallr/samples/image_04.png'` | 24.7 ± 2.2 | +1392.2% | (14.9x slower) |

> 🏆 Winner: Wallr: 14.53× faster than awww (1.7 ms vs 24.7 ms).

## 🔄 Distinct-image switch latency

This measures changing to several different images.
🏆 Each image below is its own round with its own winner.

### 🖼️ image_04.png
| Command | Mean Wall Time [ms] | Change | Factor |
|:---|---:|---:|:---|
| `wallr set '/home/programmersd/Projects/github/wallr/samples/image_04.png' --duration 0 --no-theme` | 2.2 ± 1.0 |  |  |
| `awww img '/home/programmersd/Projects/github/wallr/samples/image_04.png'` | 25.6 ± 2.6 | +1042.1% | (11.4x slower) |

> 🏆 Winner: Wallr: 11.64× faster than awww (2.2 ms vs 25.6 ms).
### 🖼️ image_05.png
| Command | Mean Wall Time [ms] | Change | Factor |
|:---|---:|---:|:---|
| `wallr set '/home/programmersd/Projects/github/wallr/samples/image_05.png' --duration 0 --no-theme` | 1.8 ± 0.7 |  |  |
| `awww img '/home/programmersd/Projects/github/wallr/samples/image_05.png'` | 25.2 ± 3.0 | +1335.4% | (14.4x slower) |

> 🏆 Winner: Wallr: 14.00× faster than awww (1.8 ms vs 25.2 ms).
### 🖼️ image_06.png
| Command | Mean Wall Time [ms] | Change | Factor |
|:---|---:|---:|:---|
| `wallr set '/home/programmersd/Projects/github/wallr/samples/image_06.png' --duration 0 --no-theme` | 1.3 ± 0.4 |  |  |
| `awww img '/home/programmersd/Projects/github/wallr/samples/image_06.png'` | 23.6 ± 1.8 | +1773.0% | (18.7x slower) |

> 🏆 Winner: Wallr: 18.15× faster than awww (1.3 ms vs 23.6 ms).

## 🧘 Post-workload resident state

This shows memory and CPU after all wallpaper changes finish.

| Program | RAM (MiB) | CPU | Threads |
|:--|--:|--:|--:|
| Wallr | 29.2 | 2.5% | 3 |
| awww | 2.4 | 0.5% | 1 |

| Program | RSS (MiB) | PSS (MiB) | Anonymous (MiB) | Private (MiB) | Non-anonymous RSS (MiB) |
|:--|--:|--:|--:|--:|--:|
| Wallr | 29.2 | 22.1 | 5.4 | 20.5 | 23.8 |
| awww | 2.4 | 0.4 | 0.1 | 0.3 | 2.3 |

> 🏆 Winner (post-workload RAM): awww: 12.22× leaner than Wallr (2.4 MiB vs 29.2 MiB).
> ℹ️ CPU snapshot (post-workload): Wallr 2.5% vs awww 0.5% (instant sample, noisy).

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
