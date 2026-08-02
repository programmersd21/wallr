# Video wallpapers

Wallr plays video files natively on the background layer. The daemon decodes the stream with FFmpeg and presents each frame through the same wgpu pipeline used for images, so transitions, per-monitor scaling, and theme generation behave identically for video and stills.

## Supported formats

MP4, WebM, MKV, MOV, and AVI. Anything FFmpeg can demux is a candidate; the FFmpeg libraries must be present when Wallr is built.

## Decoding

- Hardware acceleration is selected automatically: VAAPI on Intel and AMD, NVDEC on NVIDIA, with a software fallback.
- `video.hw_decode` forces `auto`, `vaapi`, `nvdec`, or `software`.
- `video.preferred_gpu` controls adapter selection when both integrated and discrete GPUs are present.
- Frames are decoded ahead into a small bounded queue and presented on PTS timing, so playback stays in sync without buffering the whole file.
- `wallpaper.loop_video` (default `true`) restarts the stream when it ends, producing a continuous loop.

## Playback control

Control runs through the IPC channel:

```bash
wallr ipc pause              # pause video or GIF playback
wallr ipc resume             # resume playback
wallr ipc seek 1:30          # seek to 1 minute 30 seconds (HH:MM:SS or seconds)
wallr ipc info               # version, GPU, decoder, and position
```

## Configuration

```yaml
wallpaper:
  loop_video: true
  mute: true

video:
  hw_decode: "auto"          # auto, vaapi, nvdec, software
  preferred_gpu: "auto"      # auto, integrated, discrete, or adapter name
  preload_frames: 2          # frames decoded ahead of the playhead
```

## Requirements

Hardware acceleration needs the usual driver files: `/dev/dri/renderD128` for Mesa VAAPI and `/dev/nvidia0` for NVIDIA NVDEC. Without them, software decoding still works but uses more CPU; 1080p H.264 software decode typically costs 10-20% of one core, while hardware decode keeps the daemon near idle.

## Troubleshooting

- Video not showing: confirm the compositor implements `wlr-layer-shell` (see [compositor-support.md](compositor-support.md)), then test a still image with `wallr set image.jpg`.
- Inspect the active decoder and GPU with `wallr ipc info`.
- When hardware acceleration misbehaves, force software decoding with `video.hw_decode: software`.
