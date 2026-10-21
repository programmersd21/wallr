# transitions

Choose a transition with `wallr set <path> --effect <name>`. Without an
explicit effect, wallr uses a linear crossfade.

| effect | aliases | behavior |
|:--|:--|:--|
| `fade` | `simple` | crossfade |
| `wipe` | | directional reveal |
| `slide` | `left`, `right`, `top`, `bottom` | reveal from an edge |
| `wave` | | reveal with a moving edge |
| `grow` | `center`, `any` | expand from a point |
| `outer` | | contract toward a point |

`any` chooses a random origin for a circular transition. `random` chooses a
random transition.

## options

- `-d, --duration <time>`: length, such as `800ms` or `1.2s`.
- `-o, --origin <preset|x,y>`: origin such as `center`, `top_left`, or a
  normalized coordinate pair.
- `-a, --angle <degrees>`: sweep angle for `wipe`.
- `--direction <x,y>`: direction for `wipe` or `slide`.
- `--easing <curve>`: `linear`, `ease_in`, `ease_out`, `ease_in_out`, or
  `bezier`.
- `--from <value>` and `--to <value>`: fade endpoints.
- `--frequency <hz>` and `--amplitude <value>`: wave parameters.
- `--softness <value>`: softness of the wipe edge.

```sh
wallr set ~/Pictures/wallpaper.png -e grow -o center -d 850ms
wallr set ~/Pictures/wallpaper.png -e wipe -a 45
wallr set ~/Pictures/wallpaper.png --duration 0 --no-theme
```
