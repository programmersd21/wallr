# architecture

`wallr` is the command-line client. `wallr-daemon` owns the wayland
connection, wallpaper state, rendering, and media playback. The client sends
commands over a unix socket and starts the daemon directly when configured.
The daemon uses SCTK 0.21 for wayland integration.

```text
wallr client ── unix socket ── wallr-daemon ── wayland compositor
                                  ├─ wl_shm for static images
                                  └─ wgpu for transitions and live media
```

## startup and idle

The client avoids creating an async runtime for simple local commands such as
`wallr config path`. The daemon uses a current-thread tokio runtime. Static
wallpapers use `wl_shm` when the surface protocol allows it; transitions, gif,
video, and surfaces already using the gpu path use wgpu. Settled wallpapers do
not run a frame loop.

## images and transitions

Static images without transitions use bounded `wl_shm` buffers. Image sizing
and scaling are shared with the gpu renderer. A bounded compressed cache can
avoid decoding a recently used image again. Shared-memory buffers are reused
after the compositor releases them.

Transitions use persistent wgpu resources and a fullscreen shader. Progress
follows elapsed time rather than frame count. Replacing a wallpaper cancels
obsolete transition work.

## live media

GIF frames follow their timing metadata. FFmpeg decodes video using a bounded
frame queue and timestamp-based presentation. Old frames may be dropped when
playback falls behind. Requested hardware decoding is reported as active only
after a hardware frame is received; software fallback remains available.

## outputs and state

Each output has its own wayland surface, scaling, and render state. The daemon
tracks output changes and stores the last successfully applied wallpaper for
restore after restart. Rendering resources are reused until output dimensions
or media format require a change.

## crates

- `wallr-common`: shared cli, configuration, effects, and ipc types.
- `wallr-core`: daemon, wayland integration, rendering, playback, and themes.
- `wallr`: the client and daemon executable entry points.
