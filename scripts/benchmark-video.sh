#!/usr/bin/env bash
set -euo pipefail

# Reproducible decoder benchmark. This measures decode/transfer behavior, not
# compositor presentation or GPU render time.

root_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
input=${1:-"$root_dir/assets/demo.mkv"}
report=${BENCHMARK_REPORT:-"$root_dir/benchmarks/video-$(date -u +%Y%m%d-%H%M%S).md"}

[[ -s "$input" ]] || { echo "error: missing video: $input" >&2; exit 2; }
mkdir -p "$(dirname "$report")"

run_probe() {
    local backend=$1
    cargo run -q -p wallr-core --example video_probe --release -- "$input" "$backend" 2>&1
}

software=$(run_probe software)
vaapi=$(run_probe vaapi || true)

frames=$(awk '/^decoded / {print $2; exit}' <<<"$software")
software_rate=$(sed -n 's/^decoded .* (\([0-9.]*\) fps,.*/\1/p' <<<"$software" | head -n1)
software_state=$(sed -n 's/^active backend: \([^ ]*\) (state:.*/\1/p' <<<"$software" | head -n1)
vaapi_rate=$(sed -n 's/^decoded .* (\([0-9.]*\) fps,.*/\1/p' <<<"$vaapi" | head -n1)
vaapi_state=$(sed -n 's/^active backend: \([^ ]*\) (state:.*/\1/p' <<<"$vaapi" | head -n1)
software_drops=$(awk -F'dropped frames: ' '/^active backend:/ {gsub(/\).*/, "", $2); print $2; exit}' <<<"$software")
vaapi_drops=$(awk -F'dropped frames: ' '/^active backend:/ {gsub(/\).*/, "", $2); print $2; exit}' <<<"$vaapi")

{
    echo "# Wallr video decoder benchmark"
    echo
    echo "Machine-specific decoder measurements. This report does not measure compositor presentation or GPU render time."
    echo
    printf -- '- Date (UTC): %s\n' "$(date -u '+%Y-%m-%d %H:%M:%S')"
    printf -- '- Host: %s\n' "$(hostname)"
    printf -- '- Input: `%s`\n' "$input"
    printf -- '- Wallr version: %s\n' "$(cargo run -q -p wallr -- --version 2>/dev/null)"
    echo
    echo "| Backend | Frames | Throughput (fps) | Decoder state | Dropped frames |"
    echo "|:--|--:|--:|:--|--:|"
    printf '| Software | %s | %s | %s | %s |\n' "${frames:-unavailable}" "${software_rate:-unavailable}" "${software_state:-unavailable}" "${software_drops:-unavailable}"
    printf '| VAAPI | %s | %s | %s | %s |\n' "${frames:-unavailable}" "${vaapi_rate:-unavailable}" "${vaapi_state:-unavailable}" "${vaapi_drops:-unavailable}"
} >"$report"

echo "Report saved to: $report"
