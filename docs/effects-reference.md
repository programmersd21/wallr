# Effects reference

Built-in effects are `fade`, `blur`, `wipe`, `slide`, `zoom`, `pixelate`, `ripple`, `dissolve`, `wave`, `grow`, and `outer`. Each is a typed YAML variant with its own parameters; use `wallr validate` to catch invalid values.

The default bundled package is `animations/minimal/minimal.yaml`: a restrained fade and blur crossfade. More visual effects are opt-in under `animations/`.

## awww-compatible transition model

Wallr follows the transition model used by [awww](https://codeberg.org/LGFae/awww): a transition is rendered between the retained outgoing wallpaper and the decoded incoming wallpaper, at the requested frame rate, with no synthetic black image inserted between them. `fade` is the polished crossfade equivalent of awww's `fade`; `left`, `right`, `top`, `bottom`, `wipe`, `wave`, `grow`, `center`, and `outer` cover its directional and shape-reveal family. `any` chooses a random circular reveal origin, while `random` chooses a transition family. Wallr keeps the source textures on the GPU and applies easing in WGSL, so the transition remains continuous instead of stepping individual RGB bytes.

## Fixed-position guarantee

Both wallpapers are mapped to their own stable `fill` crop for the entire transition. Wallr never reuses the incoming image's crop for the outgoing one. Every built-in masked effect uses a true screen-space circle; `direction` and `angle` choose the circle's edge origin instead of producing a straight sweep. This is deliberate: a background should feel like it is changing state, not flying across the desktop.

`zoom` is therefore a stationary focus crossfade in the built-in renderer. It keeps the familiar name for package compatibility but does not pan or magnify the full wallpaper. Raw shaders and custom effects remain available when a package explicitly wants image deformation.

The important quality rules are:

- Keep the previous wallpaper alive until the new transition has presented its final frame.
- Use `fade` or `fade` + `blur` for a restrained default. Reserve `wave`, `ripple`, and `dissolve` for images where their motion has a clear visual purpose.
- Use `ease_out` for a reveal and `ease_in_out` for a symmetric crossfade. `linear` is intentionally blunt and is best reserved for progress-like wipes.
- Tune `duration` independently; the daemon paces one frame per vsync, so a transition lasts exactly its configured duration on any refresh rate. A harsh curve or too much motion cannot be compensated by a shorter duration.

## Circular reveal system

`wipe`, `slide`, `pixelate`, `dissolve`, `wave`, `ripple`, `grow`, and `outer` share one aspect-correct radial mask. Their names and parameters remain compatible with packages, but none creates a triangle, a diagonal wedge, a rectangular sweep, or random pixel cells. `direction` and `angle` place the circle at a screen edge; `origin` chooses it directly. `fade`, `blur`, and the stationary `zoom` are deliberately full-frame blends.
