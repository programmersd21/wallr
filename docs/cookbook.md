# examples

```sh
# switch immediately
wallr set ~/Pictures/wallpaper.png --duration 0 --no-theme

# transition from a corner
wallr set ~/Pictures/wallpaper.png -e grow -o bottom_right -d 850ms

# angled wipe
wallr set ~/Pictures/wallpaper.png -e wipe -a 45 -d 800ms

# target one output
wallr set ~/Pictures/wallpaper.png --monitor DP-1

# control live playback
wallr ipc pause
wallr ipc resume
wallr ipc seek 1:30
wallr ipc info
```

See the [cli reference](cli-reference.md) for all commands and options.
