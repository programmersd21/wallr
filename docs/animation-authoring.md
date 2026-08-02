# Animation Authoring Guide

`wallr` supports customizable animation packages defined via declarative YAML specifications and WGSL shaders.

Every package must declare `duration`; declarative `custom_effects` are preferred
for new per-pixel effects before using raw WGSL. See [custom-effects.md](custom-effects.md).

---

## Animation Spec Specification (`animation.yaml`)

```yaml
name: "smooth/crossfade"
duration: "2000ms"

effects:
  - fade:
      from: 0.0
      to: 1.0
  - blur:
      from: 20.0
      to: 0.0
```

---

## Supported Effects

Every effect is a transition between the current wallpaper and the new one. All effects accept `easing`; the origin/position-sensitive effects also accept `origin`; `direction` and `angle` place circular reveals at an edge rather than creating a straight sweep.

| Effect | Type | Parameters | Defaults |
|---|---|---|---|
| `fade` | Transition | `from`, `to`, `easing` | `from: 0.0, to: 1.0` |
| `blur` | Transition | `from`, `to`, `easing` | `from: 20.0, to: 0.0` |
| `wipe` | Circular reveal | `direction`, `angle`, `softness`, `easing` | `direction: left, softness: 0.12` |
| `slide` | Circular reveal | `direction`, `easing` | `direction: left` |
| `zoom` | Transition | `from`, `to`, `origin`, `easing` | `from: 1.08, to: 1.0, origin: center` |
| `pixelate` | Transition | `from`, `to`, `easing` | `from: 64.0, to: 1.0` |
| `ripple` | Transition | `origin`, `frequency`, `amplitude`, `speed`, `easing` | `frequency: 12.0, amplitude: 0.03, speed: 5.0, origin: center` |
| `dissolve` | Transition | `scale`, `softness`, `easing` | `scale: 4.0, softness: 0.05` |
| `wave` | Circular wave reveal | `frequency`, `amplitude`, `angle`, `easing` | `frequency: 3.0, amplitude: 0.05` |
| `grow` | Transition | `origin`, `easing` | `origin: center` |
| `outer` | Transition | `origin`, `easing` | `origin: center` |

### `easing`

All effects accept an easing curve that is applied to the transition progress before any parameter is evaluated:

| Value | Curve |
|---|---|
| `linear` | Constant velocity |
| `ease_in` | Cubic ease-in (accelerating) |
| `ease_out` | Cubic ease-out (decelerating) |
| `ease_in_out` | Cubic ease-in-out (default) |
| `emphatic` | Small overshoot/settle |
| `spring` | Damped spring |

### `origin`: positioning & layout

The effects `zoom`, `ripple`, `grow`, and `outer` anchor to an origin. It accepts a named preset **or** a custom normalized `x,y` pair. Coordinates are in screen space with the origin at the **bottom-left** (`0,0` = bottom-left, `1,1` = top-right), like `swww`:

| Preset | `x, y` | Position |
|---|---|---|
| `top_left` | `0, 1` | Top-left corner |
| `top` | `0.5, 1` | Top edge center |
| `top_right` | `1, 1` | Top-right corner |
| `left` | `0, 0.5` | Left edge center |
| `center` | `0.5, 0.5` | Screen center |
| `right` | `1, 0.5` | Right edge center |
| `bottom_left` | `0, 0` | Bottom-left corner |
| `bottom` | `0.5, 0` | Bottom edge center |
| `bottom_right` | `1, 0` | Bottom-right corner |

Custom positions use `origin: custom: [0.25, 0.75]` (or `custom: [0.0, 1.0]` for top-left, block form shown below). Note the Y axis is flipped relative to the `x,y` presets above; e.g. `top_left` in YAML equals the CLI form `--origin 0,1`.

```yaml
- grow:
    origin:
      custom: [0.0, 1.0]
```

> `grow` expands a *true circle* from the origin (aspect-corrected in pixel space, never an oval) whose final radius covers the farthest screen corner. `outer` is the inverse: a circle that shrinks onto the origin from the screen edges. `ripple` expands a circular wave from the origin.

### `direction` / `angle`: circular origin

`wipe`, `slide`, and `wave` use a circular reveal. Direction and angle choose where the circle begins:

- `direction` is one of `left`, `right`, `up`, `down` (wipe/slide); it selects the edge from which the circular reveal expands.
- `angle` (degrees, `0` = right edge, `90` = top edge, counterclockwise) can be used instead on `wipe`/`wave` and overrides `direction` when both are set.

```yaml
# Diagonal origin: the circular wipe expands from the top-left edge
- wipe:
    angle: 135
    softness: 0.08
```

---

## Per-Effect YAML Examples

```yaml
name: "examples/positioning"

effects:
  # Fade with custom range and easing
  - fade:
      from: 0.2
      to: 1.0
      easing: ease_out

  # Radial blur from 40px to 0
  - blur:
      from: 40.0
      to: 0.0

  # Soft circular wipe from the left edge
  - wipe:
      direction: left
      softness: 0.02

  # Slide the new image up
  - slide:
      direction: up
      easing: ease_in_out

  # Zoom anchored at the left edge
  - zoom:
      origin:
        custom: [0.0, 0.5]
      from: 1.4
      to: 1.0

  # Pixelate back in
  - pixelate:
      from: 32.0
      to: 1.0

  # Ripple from an arbitrary point
  - ripple:
      origin:
        custom: [0.25, 0.75]
      frequency: 15.0
      amplitude: 0.05
      speed: 8.0

  # Organic dissolve, fine cells
  - dissolve:
      scale: 12.0
      softness: 0.08

  # Circular wave originating at 45 degrees
  - wave:
      angle: 45
      frequency: 5.0
      amplitude: 0.08

  # Circular reveal from the top-right corner
  - grow:
      origin:
        custom: [1.0, 1.0]
      easing: ease_out

  # Shrinking circle onto the bottom center
  - outer:
      origin:
        custom: [0.5, 0.0]
```

---

## CLI ⇄ YAML Equivalence

Everything declarable in YAML is also settable from the CLI on `wallr set` / `wallr img` / `wallr preview`:

| YAML key | CLI flag |
|---|---|
| `fade: {from, to}` → effect name | `--effect fade --from 0.2 --to 1.0` |
| `duration: "1200ms"` | `--duration 1.2s` |
| `easing` | `--easing ease_out` |
| `origin: custom: [x, y]` / `center` | `--origin 0.25,0.75` / `--origin top_right` |
| `direction: right` | `--direction 1,0` |
| `angle: 45` | `--angle 45` |
| `frequency` / `amplitude` / `speed` | `--frequency 5 --amplitude 0.08 --speed 8` |
| `softness` / `scale` | `--softness 0.02` / `--scale 12` |

CLI flags always override the package values. See [cli-reference.md](cli-reference.md) for the full flag list.

---

## Package Extension & Inheritance

Packages can extend existing animation packages using the `extends` property:

```yaml
name: "my-theme/smooth-slide"
extends:
  - "smooth/crossfade"

effects:
  - slide:
      direction: right
      easing: ease_in_out
```

---

## Custom WGSL Shaders

You can author custom shaders by passing a custom WGSL shader file:

```yaml
name: "custom/shader-effect"
effects:
  - shader:
      file: "shaders/custom.wgsl"
      uniforms:
        strength: 0.8
```

Validate your animation specification using the CLI:

```bash
wallr validate ~/.config/wallr/animations/custom.yaml
```
