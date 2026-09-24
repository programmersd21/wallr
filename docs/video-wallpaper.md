# Video wallpapers

Wallr plays video files natively on the background layer. The daemon decodes the stream with FFmpeg and presents each frame through the same wgpu pipeline used for images, so transitions, per-monitor scaling, and theme generation behave identically for video and stills.

## Supported formats

MP4, WebM, MKV, MOV, and AVI. Anything FFmpeg can demux is a candidate; the FFmpeg libraries must be present when Wallr is built.

## Decoding

- Hardware acceleration mode can be set via `video.hw_decode`:
  - `auto` (default): probes **only the hardware this machine has**.
    NVIDIA GPUs try NVDEC first; every other system goes straight to
    VAAPI (or software, when no render node exists). NVDEC is never
    probed without NVIDIA hardware, so AMD/iGPU users never see a
    "Cannot load libcuda.so.1" warning.
  - `vaapi`, `nvdec`, or `videotoolbox`: tries the specified backend first, then falls back to software if unavailable
  - `software`: uses software-only decoding without attempting hardware backends
- `video.preferred_gpu` controls adapter selection when both integrated and discrete GPUs are present.
- Frames are decoded into a small bounded latest-frame queue and presented on PTS timing. Temporary queue pressure drops stale frames instead of terminating the decoder or allowing memory growth.
- `wallpaper.loop_video` (default `true`) restarts the stream when it ends, producing a continuous loop.
- `wallr ipc info` distinguishes hardware negotiation, active hardware frames, software decoding, software fallback, and decoder failure. A backend is not reported as active until a hardware frame has actually been received.

## Playback control

Control runs through the IPC channel and works for both video and GIF wallpapers:

```bash
wallr ipc pause              # pause video or GIF playback
wallr ipc resume             # resume playback
wallr ipc seek 1:30          # seek to 1 minute 30 seconds (HH:MM:SS or seconds)
wallr ipc info               # version, GPU, decoder, and position
```

All commands support `--monitor` to target a specific output. Without `--monitor`, pause/resume/seek applies to all connected outputs.

## Configuration

```yaml
wallpaper:
  loop_video: true
  mute: true

video:
  hw_decode: "auto"          # auto, vaapi, nvdec, software
  preferred_gpu: "auto"      # auto, integrated, discrete, or adapter name
  preload_frames: 2          # frames decoded ahead of the playhead (1..=3)
```

`loop_video: false` plays the file once and then holds the final frame until
the wallpaper changes; `wallr ipc seek` restarts playback from the new
position.

## Robustness

- Transient FFmpeg errors (corrupt packets, readback hiccups, conversion
  failures) skip the affected frame instead of killing the decoder.
- A stalled or dead decoder stops playback after a grace period rather
  than freezing the frame at a burning CPU loop.
- Hardware decoding uses FFmpeg pixel-format negotiation. Device creation
  alone is not treated as proof that hardware frames are being produced.
- `wallr ipc info` reports dropped stale frames when the renderer cannot keep
  up with the decoder.
- Mid-stream resolution changes recreate only the video conversion
  resources; unchanged formats reuse them every frame.
- A slow software decoder gets up to five seconds to produce the first
  frame before the fallback black start.
- `loop_video` is honored: no hidden rewinding when disabled.

## Requirements

Hardware acceleration needs the usual driver files: `/dev/dri/renderD128` for Mesa VAAPI and `/dev/nvidia0` for NVIDIA NVDEC. Without them, software decoding still works but uses more CPU; 1080p H.264 software decode typically costs 10-20% of one core, while hardware decode keeps the daemon near idle.

FFmpeg does not need to be present at runtime for the official release binaries; they are built with a statically linked FFmpeg. Source builds link the system FFmpeg dynamically and require the development libraries (`libavcodec-dev`, `libavformat-dev`, `libavutil-dev`, `libswscale-dev`) and a system FFmpeg ABI that matches what the binary was built against.

## Troubleshooting

- Video not showing: confirm the compositor implements `wlr-layer-shell` (see [compositor-support.md](compositor-support.md)), then test a still image with `wallr set image.jpg`.
- Inspect the active decoder and GPU with `wallr ipc info`.
- When hardware acceleration misbehaves, force software decoding with `video.hw_decode: software`.
