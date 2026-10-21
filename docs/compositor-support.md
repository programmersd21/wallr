# compositor support

wallr needs a wayland compositor that supports `wlr-layer-shell`.

| compositor | notes |
|:--|:--|
| hyprland | supported |
| sway | supported |
| niri | supported; see the rule below |
| mangowm | supported |
| kde plasma wayland | supported when layer-shell is available |
| gnome/mutter | unsupported; it does not provide the required protocol |

On niri, place the wallpaper layer behind the desktop:

```kdl
layer-rule {
    match namespace="^wallr$"
    place-within-backdrop true
}
```

Compositor behavior can vary by version. Include the compositor version and
`wallr doctor` output when reporting a problem.
