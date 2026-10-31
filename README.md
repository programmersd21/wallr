<div align="center">

# wallr

wallpaper engine for wayland, with gpu transitions, gif and video playback,
and optional theme updates.

<p align="center">
  <a href="https://github.com/programmersd21/wallr"><img src="https://raw.githubusercontent.com/programmersd21/wallr/main/assets/demo.gif" alt="wallr demo" width="800"></a>
</p>

<p align="center">
  <a href="https://github.com/programmersd21/wallr/stargazers"><img src="https://img.shields.io/github/stars/programmersd21/wallr?style=for-the-badge&logo=github&logoColor=f9e2af&labelColor=11111b&color=f9e2af" alt="github stars"></a>
  <a href="https://github.com/programmersd21/wallr/releases/latest"><img src="https://img.shields.io/github/v/release/programmersd21/wallr?style=for-the-badge&logo=github&logoColor=a6e3a1&labelColor=11111b&color=a6e3a1" alt="latest release"></a>
  <a href="https://aur.archlinux.org/packages/wallr-bin"><img src="https://img.shields.io/aur/version/wallr-bin?style=for-the-badge&logo=archlinux&logoColor=cba6f7&labelColor=11111b&color=cba6f7" alt="aur version"></a>
  <a href="https://aur.archlinux.org/packages/wallr-bin"><img src="https://img.shields.io/aur/votes/wallr-bin?style=for-the-badge&logo=archlinux&logoColor=f5c2e7&labelColor=11111b&color=f5c2e7" alt="aur votes"></a>
  <a href="https://crates.io/crates/wallr"><img src="https://img.shields.io/crates/v/wallr?style=for-the-badge&logo=rust&logoColor=fab387&labelColor=11111b&color=fab387" alt="crates.io version"></a>
  <a href="https://www.rust-lang.org"><img src="https://img.shields.io/badge/msrv-1.90-94e2d5?style=for-the-badge&logo=rust&logoColor=94e2d5&labelColor=11111b&color=94e2d5" alt="minimum supported rust version"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-313244?style=for-the-badge&labelColor=11111b&color=313244" alt="license"></a>
</p>

</div>

## table of contents

- [background](#background)
- [features](#features)
- [why wallr](#why-wallr)
- [when not to use wallr](#when-not-to-use-wallr)
- [pros and cons](#pros-and-cons)
- [how it compares](#how-it-compares)
- [requirements](#requirements)
- [installation](#installation)
- [build from source](#build-from-source)
- [quick start](#quick-start)
- [configuration](#configuration)
- [cli reference](#cli-reference)
- [effects and transitions](#effects-and-transitions)
- [video wallpapers](#video-wallpapers)
- [animated wallpapers](#animated-wallpapers)
- [scaling modes](#scaling-modes)
- [multi-monitor and hotplug](#multi-monitor-and-hotplug)
- [theme integration](#theme-integration)
- [hooks and reloads](#hooks-and-reloads)
- [ipc and daemon](#ipc-and-daemon)
- [performance](#performance)
- [security and safety](#security-and-safety)
- [troubleshooting](#troubleshooting)
- [documentation](#documentation)
- [contributing](#contributing)
- [license](#license)
- [faq](#faq)

## background

wallr is a native wallpaper engine for wayland. it is written in rust and uses
a small client plus a long-lived daemon. the daemon owns background
layer-shell surfaces, renders transitions with wgpu, and plays animated or
video wallpapers without asking the compositor to do wallpaper-specific work.

the project is intentionally focused. it does not try to be a desktop shell,
a widget framework, or a compositor plugin. it sets wallpapers, switches them
with transitions, plays media, and optionally updates theme files.

if you are coming from tools such as swww, awww, hyprpaper, or
swaybg, wallr aims to give you a single command-line workflow with animated
and video support, multi-monitor handling, and optional theme integration.

## features

- static image wallpapers through a shared-memory fast path
- gpu transitions for images, gifs, and videos
- animated gif playback with bounded frame caching
- video playback through FFmpeg, with software fallback
- multi-monitor support and output hotplug handling
- per-output scaling modes: fill, fit, stretch, center, and tile
- optional theme generation with matugen, wallust, or pywal
- user hooks and reload commands
- ipc commands for pause, resume, seek, blank, restore, and status
- shared-memory presentation for static images
- bounded decoder queues and seek error propagation
- deterministic monitor listing
- doctor diagnostics for wayland and theme tools
- release binaries with static FFmpeg

## why wallr

wallr is useful when you want more than a static image but do not want a heavy
desktop shell or a compositor-specific wallpaper plugin. it keeps the wallpaper
as a background layer surface, handles multiple outputs, and can switch media
without restarting the daemon.

it is especially useful for:

- animated wallpapers on wlroots-based compositors
- short video loops as desktop backgrounds
- themed desktops where wallpaper changes trigger color generation
- multi-monitor setups
- users who want a command-line driven workflow
- systems where predictable memory use matters

## when not to use wallr

wallr may not be the right choice if:

- your compositor does not support `wlr-layer-shell`
- you only need a static image and prefer the smallest possible tool
- you need x11 support
- you need a desktop shell with widgets or panels
- your hardware cannot run a wayland compositor with gpu support
- you need guaranteed universal hardware decoding on every driver

in those cases, simpler tools may be a better fit.

## pros and cons

### pros

- native wayland integration instead of x11 compatibility layers
- gpu-accelerated transitions and animation
- bounded memory budgets for decoded video frames
- shared-memory path for static images
- multi-output and hotplug aware
- optional theme integration
- command-line and ipc driven workflow
- open source and packaged for arch, nix, and crates.io
- diagnostics command for common setup problems
- release binaries with static FFmpeg

### cons

- requires a wayland compositor with `wlr-layer-shell`
- hardware decoding depends on drivers and supported backends
- video playback can use significant cpu or memory on low-end systems
- some features depend on external tools such as FFmpeg or theme providers
- compositor and gpu behaviour must be verified on each target system
- animated and video wallpapers use more resources than static images
- not every compositor has been tested

## how it compares

wallr is not trying to replace every wallpaper tool. it occupies a specific
space: native wayland, animated and video wallpapers, transitions, and theme
integration.

compared with static-image tools, wallr does more work and uses more resources,
but gives smoother switching and richer media support.

compared with heavier desktop environments, wallr stays small and focused on
wallpaper behaviour.

compared with compositor-specific plugins, wallr is portable across compositors
that implement `wlr-layer-shell`.

## requirements

- a wayland compositor with `wlr-layer-shell`
- rust 1.90 or newer for source builds
- FFmpeg development libraries for dynamic source builds
- official static release binaries do not require a separate FFmpeg install
- for static builds, the Linux tools listed by
  `scripts/install-static-ffmpeg.sh`

## installation

### cargo

```sh
cargo install wallr
```

this installs the `wallr` client and `wallr-daemon`.

### arch linux

```sh
yay -S wallr-bin
```

### nix

```sh
nix run github:programmersd21/wallr
```

### release archive

download the latest archive from the
[releases page](https://github.com/programmersd21/wallr/releases), extract it,
and put both `wallr` and `wallr-daemon` on your `PATH`.

## build from source

```sh
git clone https://github.com/programmersd21/wallr.git
cd wallr
cargo build --workspace --release --locked
```

for a static FFmpeg build, run `./scripts/install-static-ffmpeg.sh`, then export
the variables it prints and run the same build command.

more setup and project conventions are in [contributing](CONTRIBUTING.md).

## quick start

```sh
wallr set wallpaper.jpg
wallr set wallpaper.jpg -e grow -o center -d 850ms
wallr set video.mp4 --fps 30
wallr ipc status
```

`wallr set` starts the daemon automatically when `daemon.auto_start` is enabled.
to run it manually, use `wallr daemon` or start `wallr-daemon` directly. see
the [cli reference](docs/cli-reference.md).

## configuration

the default file is `~/.config/wallr/config.yaml`:

```yaml
wallpaper:
  mode: fill
  loop_video: true

theme:
  provider: matugen

daemon:
  auto_start: true
  max_fps: 60
```

see the [configuration reference](docs/config-reference.md) for all options.

## cli reference

common commands:

```sh
wallr set <path>
wallr set <path> -e grow -o center -d 850ms
wallr set video.mp4 --fps 30
wallr reload
wallr monitor list
wallr monitor current
wallr ipc pause
wallr ipc resume
wallr ipc seek 1:30
wallr ipc status
wallr doctor
wallr quit
```

use `--monitor <output>` to target one output where supported.

## effects and transitions

wallr supports fade, wipe, slide, wave, grow, and outer transitions. duration,
origin, angle, direction, easing, and effect parameters can be overridden from
the command line.

examples:

```sh
wallr set wallpaper.jpg -e wipe -a 45 -d 700ms
wallr set wallpaper.jpg -e grow -o center -d 850ms
wallr set wallpaper.jpg -e wave -o 0.25,0.75
```

see the [effects reference](docs/effects-reference.md).

## video wallpapers

video playback uses FFmpeg and supports software fallback. per-wallpaper fps
caps and scaling modes are available.

```sh
wallr set video.mp4 --fps 30
wallr ipc seek 1:30
wallr ipc pause
wallr ipc resume
```

the decoder queues a bounded number of frames. if playback falls behind, stale
frames can be dropped. presentation follows video timestamps. see
[video-wallpaper.md](docs/video-wallpaper.md).

## animated wallpapers

animated gif wallpapers are decoded on demand and cached within a bounded
budget. extension handling is case-insensitive.

```sh
wallr set animation.gif
```

animated wallpapers support transitions, pause, resume, and theme frame
extraction.

## scaling modes

wallr supports:

- fill
- fit
- stretch
- center
- tile

fill is the default. fit preserves aspect ratio and leaves unused areas
defined by the compositor background. center renders at native size.

## multi-monitor and hotplug

wallr creates a surface for each detected output and keeps output state
isolated. outputs can be targeted by name.

```sh
wallr monitor list
wallr set wallpaper.jpg --monitor DP-1
wallr ipc pause --monitor HDMI-A-1
```

output addition, removal, and reconfiguration are handled by the daemon.

## theme integration

wallr can run theme providers after a wallpaper changes.

supported providers:

- matugen
- wallust
- pywal
- none

configuration:

```yaml
theme:
  provider: matugen

matugen:
  enabled: true
  mode: dark
  scheme: scheme-tonal-spot
```

theme failures are reported separately and do not hide a successful wallpaper
change.

## hooks and reloads

hooks run before, after, or on error. reload commands run after successful
wallpaper changes.

```yaml
hooks:
  before: []
  after: []
  error: []

reload:
  - "pkill -USR1 some-program"
```

use `wallr reload` to reload configuration and hooks.

## ipc and daemon

wallr uses a unix socket for client-daemon communication. the daemon handles
wallpaper changes, playback, and output lifecycle.

```sh
wallr ipc status
wallr ipc info
wallr ipc blank
wallr ipc restore
```

the socket is created with restrictive permissions. oversized or malformed
requests are rejected.

## performance

wallr aims to avoid unnecessary work:

- static images can use shared-memory presentation
- gpu resources are reused where possible
- decoder queues are bounded
- seek errors propagate instead of being hidden
- idle static wallpapers do not require continuous rendering

performance depends on compositor, gpu, driver, media resolution, and settings.

## security and safety

wallr handles local media and configuration. it validates ipc request sizes,
monitor names, paths, and effect parameters. subprocesses are invoked with
argument arrays rather than shell strings where possible.

unsafe code is limited to wayland handle wrappers and FFmpeg FFI. safety
comments describe the invariants.

## troubleshooting

- run `wallr doctor` to check wayland and theme provider availability
- if `WAYLAND_DISPLAY` is not set, wallr cannot run
- if hardware decoding fails, set `video.hw_decode: software`
- if a video does not play, test a still image first, then run `wallr ipc info`
- if theme generation fails, the wallpaper change still succeeds
- static release binaries do not need a separate FFmpeg runtime install

## documentation

[cli](docs/cli-reference.md) · [configuration](docs/config-reference.md) ·
[effects](docs/effects-reference.md) · [video](docs/video-wallpaper.md) ·
[compositors](docs/compositor-support.md) · [architecture](docs/architecture.md) ·
[benchmarks](docs/benchmarks.md) · [changelog](CHANGELOG.md)

## contributing

contributions are welcome. see [CONTRIBUTING.md](CONTRIBUTING.md) for setup,
style, testing, and pull request expectations.

## license

MIT. See [LICENSE](LICENSE).

## faq

### does wallr support x11?

no. wallr is wayland-only.

### does wallr support all compositors?

no. it requires `wlr-layer-shell`, which is common on wlroots-based
compositors but not universal.

### can wallr play 4k video?

yes, if the system has enough cpu, memory, and gpu resources. performance
depends on codec, driver, and compositor.

### does wallr upload private data?

no. wallr runs locally and does not send telemetry.

### can i use wallr without theme integration?

yes. set `theme.provider: none` or use `--no-theme`.

### is wallr production ready?

wallr is actively developed. core functionality is tested, but compositor and
gpu behaviour must be verified on your system.
