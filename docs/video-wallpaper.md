# video and animated wallpapers

wallr supports video through FFmpeg and animated GIF playback. The daemon
applies output scaling and uses its rendering pipeline to display frames.

## decoding

Set `video.hw_decode` to:

- `auto` (default): try a detected compatible hardware backend, then software.
- `nvdec` or `vaapi`: request that backend, with software fallback.
- `software`: disable hardware decoding.

Hardware acceleration is reported as active only after wallr receives a
hardware-decoded frame. `wallr ipc info` shows the decoder state. NVIDIA
decoding requires a working NVIDIA driver; VAAPI requires a supported device
and driver. Software decoding remains available but can use more cpu.

The decoder queues a bounded number of frames, capped at three frames and
256 MiB of decoded frame data. Playback can hold one additional pending frame.
If playback falls behind, stale frames are dropped instead of allowing the
queue to grow. This budget covers application-owned decoded frames only;
FFmpeg internal buffers, hardware frame pools, conversion buffers, GPU
textures, and per-output renderer resources are outside it. Presentation
follows video timestamps. `wallpaper.loop_video` controls looping, and
`wallpaper.mute` keeps audio muted.

Set a per-wallpaper frame-rate ceiling with `--fps`:

```sh
wallr set video.mp4 --fps 30
```

The supported range is 1 to 240 fps. This caps presentation work; it does not
change the media's encoded frame rate.

## theme generation

Theme providers use a still frame from gif and video. Extraction does not
replace or pause live playback:

```sh
wallr set video.mp4 --theme matugen
wallr set animation.gif --theme matugen
```

The extracted frame is cached and refreshed when the source changes.

## playback controls

```sh
wallr ipc pause
wallr ipc resume
wallr ipc seek 1:30
wallr ipc info
```

Seeks wait for the decoder to confirm the operation. A failed seek returns an
error and leaves the previous playback position in place. A newer seek
supersedes a queued seek rather than being applied out of order.

Use `--monitor <output>` to target an output where supported. Without it,
playback controls apply to all outputs.

## limits and troubleshooting

wallr validates frame dimensions and memory before allocating. Actual resolution
limits depend on available memory and gpu limits. Hardware decoding does not
remove the need to transfer or render frames in all driver configurations.

If video does not play, first test a still image and run `wallr ipc info`.
For hardware issues, try `video.hw_decode: software`. Source builds need the
FFmpeg development libraries for dynamic builds. Static builds are optional;
run `scripts/install-static-ffmpeg.sh` to prepare the pinned static toolchain.
