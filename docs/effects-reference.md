# Effects reference

Wallr ships six built-in transitions. Pick one with `wallr set <path> -e <name>` and tune it with the flags below. With no `-e` flag you get a plain linear crossfade, matching awww's default `simple` feel.

| Name | Aliases | Look |
|---|---|---|
| `fade` | `simple` | Smooth crossfade between wallpapers. |
| `wipe` | | Directional reveal sweep, optionally angled. |
| `slide` | `left`, `right`, `top`, `bottom` | Directional reveal from an edge. |
| `wave` | | Wipe with an oscillating edge. |
| `grow` | `center`, `any` | Expanding circle from an origin. |
| `outer` | | Shrinking circle onto an origin. |

`any` grows or shrinks from a random point. `random` picks one of the six at random.

## Flags

| Flag | Used by | Meaning |
|---|---|---|
| `-d, --duration <TIME>` | all | Wall-clock length, e.g. `700ms`, `1s`, `1.2s`. |
| `-o, --origin <PRESET\|X,Y>` | `grow`, `outer`, `wave` | Circle origin: `center` (default), `top_left`, `top`, `top_right`, `left`, `right`, `bottom_left`, `bottom`, `bottom_right`, or normalized `x,y`. On `wave` an `--angle` derives the center from the sweep direction. |
| `-a, --angle <DEG>` | `wipe`, `slide` | Sweep angle in degrees: `0` = right-to-left, `90` = top-to-bottom, `270` = bottom-to-top. |
| `--direction <X,Y>` | `wipe`, `slide` | Direction vector, e.g. `1,0`. |
| `--easing <CURVE>` | all | `linear`, `ease_in`, `ease_out`, `ease_in_out`, `bezier`. Named effects default to `bezier` (awww-style cubic curve); with no `-e` flag the fade is plain linear. |
| `--from, --to <VAL>` | `fade` | Opacity range, default `0` to `1`. |
| `--frequency, --amplitude` | `wave` | Wave density and height. |
| `--softness <VAL>` | `wipe` | Edge feather in screen fraction, `0.002` to `0.25` (default `0.01`: a sharp, pixel-scale edge). |

## Examples

```bash
wallr set ~/Pictures/wallpaper.png -e grow -o center -d 850ms
wallr set ~/Pictures/wallpaper.png -e wipe -a 45 -d 800ms
wallr set ~/Pictures/wallpaper.png -e wave -d 900ms
wallr set ~/Pictures/wallpaper.png --duration 0 --no-theme
```

`--duration 0` skips the transition and presents the wallpaper directly.
