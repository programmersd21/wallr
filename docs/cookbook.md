# Cookbook

1. Soft crossfade: use `animations/minimal/minimal.yaml`.
2. Circular wipe: add `wipe: {direction: right, softness: 0.12}`. Direction selects the circle's edge origin; the wider feather keeps it calm and cinematic.
3. Blur and fade pair: run both at `at: 0ms` with the same duration.
4. Spring zoom: use `zoom` with `easing: spring` and an off-center origin.
5. Two-stage reveal: use timeline entries at `0ms` and `300ms`.
6. Custom scanlines: copy the `scanline` definition in `custom-effects.md`.
7. Custom glass: see `animations/apple/liquid.yaml`.
8. Custom retro treatment: see `animations/pixel/retro.yaml`.
9. Raw WGSL: use the `shader` escape hatch described in [shaders](shaders.md).

Validate every package before previewing it: `wallr validate path/to/wallr.yaml`.
