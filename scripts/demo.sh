#!/usr/bin/env bash
# Wallr transition demo: applies every built-in transition over the sample
# images and records each change, ending with a coverage summary.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_ROOT"

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
sleep 3

IMAGES=()
for image in samples/*.png; do
    [[ -f "$image" ]] && IMAGES+=("$image")
done
[[ ${#IMAGES[@]} -gt 0 ]] || { echo "no samples/*.png found" >&2; exit 1; }

# The six built-in transitions. Effects are the outer loop so every one is
# exercised exactly once per cycle regardless of how many images exist.
EFFECTS=("fade" "wipe" "slide" "wave" "grow" "outer")
PARAMS=(
    ""
    "--direction 1,0"
    "--direction 0,1"
    "--angle 45"
    "--origin bottom_right"
    "--origin top_left"
)

echo "=========================================="
echo "Starting Wallpaper Rotation Demo"
echo "=========================================="

record=()
APPLIED=0
for ((e = 0; e < ${#EFFECTS[@]}; e++)); do
    EFFECT="${EFFECTS[$e]}"
    EXTRA="${PARAMS[$e]}"
    IMAGE="${IMAGES[$((e % ${#IMAGES[@]}))]}"
    # shellcheck disable=SC2206
    EXTRA_ARGS=($EXTRA)

    echo "[record] $((e + 1))/${#EFFECTS[@]} $EFFECT ${EXTRA_ARGS[*]:-} <- $(basename "$IMAGE")"
    # shellcheck disable=SC2086 # intentional word splitting for effect flags
    if "$WALLR" set "$IMAGE" \
        --effect "$EFFECT" \
        ${EXTRA_ARGS[@]} \
        --duration 2000ms \
        --no-theme
    then
        record+=("$EFFECT")
        APPLIED=$((APPLIED + 1))
    fi
    sleep 3
done

echo "=========================================="
echo "Applied: $APPLIED transition(s)."
echo "------------------------------------------"
count=0
for EFFECT in "${EFFECTS[@]}"; do
    count=0
    for applied in "${record[@]}"; do
        [[ "$applied" == "$EFFECT" ]] && count=$((count + 1))
    done
    printf '  %-6s %sx\n' "$EFFECT" "$count"
done
echo "=========================================="