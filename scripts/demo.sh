#!/usr/bin/env bash
# Wallr transition demo: applies every built-in transition over the sample
# images, records the changes, and renders a looping GIF to assets/demo.gif.
#
# Capture modes (auto-detected):
#   wf-recorder  real 60fps video source -> transcoded to a 60fps GIF
#   grim         per-frame screenshots, encoded at the measured rate and
#                upsampled to a uniform 60fps stream

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_ROOT"

WALLR=""
if [[ -x "./target/release/wallr" ]]; then
    WALLR="./target/release/wallr"
elif [[ -x "./target/debug/wallr" ]]; then
    WALLR="./target/debug/wallr"
else
    echo "Building wallr in release mode first..."
    cargo build --release
    WALLR="./target/release/wallr"
fi

echo "Using wallr binary: $WALLR"
$WALLR doctor || true

# Stabilize the desktop before recording anything.
echo "Settling for 5 seconds..."
sleep 5

IMAGES=()
for image in samples/*.png; do
    [[ -f "$image" ]] && IMAGES+=("$image")
done
[[ ${#IMAGES[@]} -gt 0 ]] || { echo "no samples/*.png found" >&2; exit 1; }

EFFECTS=("fade" "wipe" "slide" "wave" "grow" "outer")
PARAMS=(
    ""
    "--direction 1,0"
    "--direction 0,1"
    "--angle 45"
    "--origin bottom_right"
    "--origin top_left"
)

DURATION_MS="${DEMO_DURATION_MS:-2000}"
OUT_GIF="${1:-assets/demo.gif}"
FPS="${DEMO_FPS:-60}"
DURATION_SECS=$((DURATION_MS / 1000))

mkdir -p "$(dirname "$OUT_GIF")"

if command -v wf-recorder >/dev/null && command -v ffmpeg >/dev/null; then
    CAPTURE="wf-recorder"
elif command -v grim >/dev/null && command -v ffmpeg >/dev/null; then
    CAPTURE="grim"
else
    echo "warning: no recorder (wf-recorder or grim) with ffmpeg found; running the rotation without recording."
    CAPTURE="none"
fi

RECORD_DIR="$(mktemp -d)"
trap 'rm -rf "$RECORD_DIR"' EXIT

record_transition() {
    # Sample the transition while it renders async in the background.
    if [[ "$CAPTURE" != "grim" ]]; then
        sleep $((DURATION_SECS + 1))
        return
    fi
    local started=$SECONDS
    while ((SECONDS - started < DURATION_SECS + 1)); do
        local idx frame
        idx=$(cat "$RECORD_DIR/index" 2>/dev/null || printf '0')
        frame="$RECORD_DIR/$(printf '%05d' "$idx").png"
        # Only advance on a successful grab so the PNG sequence stays gapless
        # (a skipped grim would otherwise make ffmpeg stop the sequence early).
        if grim "$frame" 2>/dev/null; then
            printf '%d\n' "$((idx + 1))" >"$RECORD_DIR/index"
        fi
    done
}

run_rotation() {
    local -a record=()
    local applied=0 e EFFECT EXTRA IMAGE
    for ((e = 0; e < ${#EFFECTS[@]}; e++)); do
        EFFECT="${EFFECTS[$e]}"
        EXTRA="${PARAMS[$e]}"
        IMAGE="${IMAGES[$((e % ${#IMAGES[@]}))]}"
        # shellcheck disable=SC2206
        local -a EXTRA_ARGS=($EXTRA)

        echo "[record] $((e + 1))/${#EFFECTS[@]} $EFFECT ${EXTRA_ARGS[*]:-} <- $(basename "$IMAGE")"
        # shellcheck disable=SC2086 # intentional word splitting for effect flags
        "$WALLR" set "$IMAGE" \
            --effect "$EFFECT" \
            ${EXTRA_ARGS[@]} \
            --duration "$DURATION_MS"ms \
            --no-theme
        record_transition
        record+=("$EFFECT")
        applied=$((applied + 1))
        sleep 1
    done

    echo "=========================================="
    echo "Applied: $applied transition(s)."
    echo "------------------------------------------"
    for EFFECT in "${EFFECTS[@]}"; do
        local count=0 tick
        for tick in "${record[@]}"; do
            [[ "$tick" == "$EFFECT" ]] && count=$((count + 1))
        done
        printf '  %-6s %sx\n' "$EFFECT" "$count"
    done
}

echo "=========================================="
echo "Starting Wallpaper Rotation Demo ($CAPTURE capture)"
echo "=========================================="

case "$CAPTURE" in
    wf-recorder)
        wf-recorder -f "$RECORD_DIR/seq.mkv" -r "$FPS" >/dev/null 2>&1 &
        rec_pid=$!
        sleep 1
        run_rotation
        kill -INT "$rec_pid" 2>/dev/null || true
        wait "$rec_pid" 2>/dev/null || true
        sleep 1
        [[ -f "$RECORD_DIR/seq.mkv" ]] || { echo "recording failed" >&2; exit 1; }
        echo "Encoding $OUT_GIF ..."
        ffmpeg -y -loglevel error -i "$RECORD_DIR/seq.mkv" \
            -vf "fps=$FPS,split[s0][s1];[s0]palettegen=stats_mode=diff[p];[s1][p]paletteuse=dither=bayer:bayer_scale=5" \
            -loop 0 "$OUT_GIF"
        ;;
    grim)
        echo "note: grim captures at its own rate; output is upsampled to ${FPS}fps."
        printf '0\n' >"$RECORD_DIR/index"
        start_sec="$SECONDS"
        run_rotation
        end_sec="$SECONDS"
        frames=$(cat "$RECORD_DIR/index")
        capture_secs=$((end_sec - start_sec))
        rate=$(awk -v f="$frames" -v s="${capture_secs:-1}" 'BEGIN { printf "%.2f", f / (s > 0 ? s : 1) }')
        echo "------------------------------------------"
        echo "Captured $frames frame(s) over ${capture_secs}s (~${rate} fps); encoding $OUT_GIF ..."
        ffmpeg -y -loglevel error -framerate "$rate" -i "$RECORD_DIR/%05d.png" \
            -vf "fps=$FPS,split[s0][s1];[s0]palettegen=stats_mode=diff[p];[s1][p]paletteuse=dither=bayer:bayer_scale=5" \
            -loop 0 "$OUT_GIF"
        ;;
    none)
        run_rotation
        ;;
esac

echo "=========================================="
echo "GIF written to: $OUT_GIF"
echo "=========================================="