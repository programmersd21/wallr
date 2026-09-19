#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd -- "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_ROOT"

DURATION_MS="${DEMO_DURATION_MS:-2000}"
FPS="${DEMO_FPS:-60}"
OUT_MKV="${1:-assets/demo.mkv}"

EFFECTS=(
    fade
    wipe
    slide
    wave
    grow
    outer
)

IMAGES=()
for image in samples/*.png; do
    [[ -f "$image" ]] && IMAGES+=("$image")
done

if ((${#IMAGES[@]} == 0)); then
    printf '%s\n' "error: no samples/*.png found" >&2
    exit 1
fi

if [[ -x ./target/release/wallr ]]; then
    WALLR=./target/release/wallr
elif [[ -x ./target/debug/wallr ]]; then
    WALLR=./target/debug/wallr
else
    printf '%s\n' "building wallr..."
    cargo build --release
    WALLR=./target/release/wallr
fi

if ! command -v wf-recorder >/dev/null 2>&1; then
    printf '%s\n' "error: wf-recorder is required" >&2
    exit 1
fi

mkdir -p -- "$(dirname -- "$OUT_MKV")"

REC_PID=""

cleanup() {
    if [[ -n "$REC_PID" ]] && kill -0 "$REC_PID" 2>/dev/null; then
        kill -INT "$REC_PID" 2>/dev/null || true
        wait "$REC_PID" 2>/dev/null || true
    fi
}

trap cleanup EXIT INT TERM

run_transition() {
    local effect="$1"
    local image="$2"

    case "$effect" in
        fade)
            "$WALLR" set "$image" \
                --effect fade \
                --duration "${DURATION_MS}ms" \
                --easing bezier \
                --no-theme
            ;;
        wipe)
            "$WALLR" set "$image" \
                --effect wipe \
                --direction 1,0 \
                --duration "${DURATION_MS}ms" \
                --easing bezier \
                --no-theme
            ;;
        slide)
            "$WALLR" set "$image" \
                --effect slide \
                --direction 0,1 \
                --duration "${DURATION_MS}ms" \
                --easing bezier \
                --no-theme
            ;;
        wave)
            "$WALLR" set "$image" \
                --effect wave \
                --angle 45 \
                --duration "${DURATION_MS}ms" \
                --easing bezier \
                --no-theme
            ;;
        grow)
            "$WALLR" set "$image" \
                --effect grow \
                --origin bottom_right \
                --duration "${DURATION_MS}ms" \
                --easing bezier \
                --no-theme
            ;;
        outer)
            "$WALLR" set "$image" \
                --effect outer \
                --origin top_left \
                --duration "${DURATION_MS}ms" \
                --easing bezier \
                --no-theme
            ;;
        *)
            printf 'error: unknown effect: %s\n' "$effect" >&2
            return 1
            ;;
    esac
}

run_rotation() {
    local i effect image
    local duration_seconds

    duration_seconds="$(awk "BEGIN { printf \"%.3f\", ${DURATION_MS} / 1000 }")"

    for i in "${!EFFECTS[@]}"; do
        effect="${EFFECTS[$i]}"
        image="${IMAGES[$((i % ${#IMAGES[@]}))]}"

        printf '[%d/%d] %-6s <- %s\n' \
            "$((i + 1))" \
            "${#EFFECTS[@]}" \
            "$effect" \
            "$(basename -- "$image")"

        run_transition "$effect" "$image"

        sleep "$duration_seconds"
        sleep 0.5
    done
}

printf '%s\n' "=========================================="
printf '%s\n' "Wallr Transition Demo"
printf '%s\n' "=========================================="
printf 'duration: %sms\n' "$DURATION_MS"
printf 'capture:  %sfps\n' "$FPS"
printf 'output:   %s\n' "$OUT_MKV"
printf '%s\n' "=========================================="

"$WALLR" doctor || true

printf '%s\n' "settling desktop..."
sleep 2

printf '%s\n' "starting wf-recorder..."

wf-recorder \
    -f "$OUT_MKV" \
    -r "$FPS" &

REC_PID=$!

sleep 3

run_rotation

printf '%s\n' "stopping recorder..."

kill -INT "$REC_PID" 2>/dev/null || true
wait "$REC_PID" 2>/dev/null || true
REC_PID=""

if [[ ! -s "$OUT_MKV" ]]; then
    printf '%s\n' "error: recording failed" >&2
    exit 1
fi

printf '%s\n' "=========================================="
printf 'MKV written to: %s\n' "$OUT_MKV"
printf '%s\n' "=========================================="
