#!/usr/bin/env bash
set -euo pipefail

# Comparable local benchmark for a live Wayland session.
# Usage: scripts/benchmark-wallpapers.sh [image ...]

if [[ "${1:-}" == "--help" || "${1:-}" == "-h" ]]; then
    cat <<'USAGE'
Usage: scripts/benchmark-wallpapers.sh [IMAGE ...]

Compare the local release Wallr daemon with awww in a live Wayland session.
Reports are written to benchmarks/ as readable Markdown tables.

Environment:
  WALLR_BIN                 Wallr release binary override
  BENCHMARK_REPORT          Explicit Markdown report path
  BENCHMARK_INCLUDE_ANIMATED=0  Skip GIF measurements
USAGE
    exit 0
fi

if [[ -z "${WAYLAND_DISPLAY:-}" || -z "${XDG_RUNTIME_DIR:-}" ]]; then
    echo "error: a live Wayland session is required" >&2
    exit 2
fi
command -v hyperfine >/dev/null || { echo "error: hyperfine is required" >&2; exit 2; }
command -v awww >/dev/null || { echo "error: awww is required" >&2; exit 2; }

root_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
wallr_bin=${WALLR_BIN:-"$root_dir/target/release/wallr"}
[[ -x "$wallr_bin" ]] || { echo "error: build Wallr release first: $wallr_bin" >&2; exit 2; }

mkdir -p "$root_dir/benchmarks"
timestamp=$(date -u +%Y%m%d-%H%M%S)
report_file=${BENCHMARK_REPORT:-"$root_dir/benchmarks/wallr-vs-awww-$timestamp.md"}
mkdir -p "$(dirname "$report_file")"
report_tmp="$report_file.tmp.$$"
# Isolate the benchmark daemon from any installed/user-managed Wallr daemon.
# The override is consumed by both the daemon and its local CLI clients.
export WALLR_SOCKET="${WALLR_SOCKET:-$XDG_RUNTIME_DIR/wallr-benchmark-$timestamp.sock}"
cleanup_report() {
    local status=$?
    if [[ -n "${local_daemon_pid:-}" ]] && kill -0 "$local_daemon_pid" 2>/dev/null; then
        kill "$local_daemon_pid" 2>/dev/null || true
    fi
    rm -f "$WALLR_SOCKET"
    if ((status == 0)); then
        mv -f "$report_tmp" "$report_file"
        cat "$report_file" >&3
    else
        cat "$report_tmp" >&3 2>/dev/null || true
        rm -f "$report_tmp"
    fi
    if [[ -n "${local_log:-}" ]]; then
        rm -f "$local_log"
    fi
    exit "$status"
}
trap cleanup_report EXIT

exec 3>&1
exec >"$report_tmp" 2>&1
echo "# Wallr vs awww benchmark"
echo
echo "> Production benchmark report for wallpaper switching performance."
echo "> Lower latency, RAM, and CPU values are better."
echo
echo "- Date (UTC): $(date -u '+%Y-%m-%d %H:%M:%S')"
echo "- Host: $(hostname)"
printf -- '- Wallr client binary: `%s`\n' "$wallr_bin"
printf -- '- Benchmark IPC socket: `%s`\n' "$WALLR_SOCKET"
echo "- Wallr version: $($wallr_bin --version 2>/dev/null || echo unknown)"
echo "- awww version: $(awww --version 2>/dev/null || echo unknown)"
echo "- Wayland display: ${WAYLAND_DISPLAY:-unknown}"
echo
echo
echo "## How to read this report"
echo
echo "- **Latency:** time required to submit a wallpaper change."
echo "- **RAM:** resident memory used by the daemon, shown in MiB."
echo "- **CPU:** processor usage sampled by the operating system."
echo "- **Relative:** how much slower the second command was than Wallr."
echo "- **Verdict:** a winner is reported only for the workload actually measured."
echo "- **Winner:** every step ends with a 🏆 winner line so you can skim the report."
echo

images=("$@")
if ((${#images[@]} == 0)); then
    images=("$root_dir/samples/image_04.png" "$root_dir/samples/image_05.png" "$root_dir/samples/image_06.png")
fi
for image in "${images[@]}"; do
    [[ -s "$image" ]] || { echo "error: missing image: $image" >&2; exit 2; }
done

wallr_pid=""
if [[ -S "$WALLR_SOCKET" ]] && "$wallr_bin" ipc status >/dev/null 2>&1; then
    for candidate in $(pgrep -x wallr || true); do
        executable=$(readlink -f "/proc/$candidate/exe" 2>/dev/null || true)
        if [[ "$executable" == "$wallr_bin" ]]; then
            wallr_pid=$candidate
            break
        fi
    done
fi

if [[ -z "$wallr_pid" ]]; then
    local_log=$(mktemp)
    "$wallr_bin" daemon >"$local_log" 2>&1 &
    local_daemon_pid=$!
    for _ in {1..50}; do
        if "$wallr_bin" ipc status >/dev/null 2>&1; then
            wallr_pid=$local_daemon_pid
            break
        fi
        sleep 0.1
    done
    if [[ -z "$wallr_pid" ]]; then
        echo "error: local release daemon did not start" >&2
        cat "$local_log" >&2
        kill "$local_daemon_pid" 2>/dev/null || true
        rm -f "$local_log"
        exit 2
    fi
fi
wallr_executable=$(readlink -f "/proc/$wallr_pid/exe")
printf -- '- Wallr daemon binary: `%s`\n' "$wallr_executable"
awww_pid=$(pgrep -n -x awww-daemon || true)
[[ -n "$wallr_pid" ]] || { echo "error: Wallr daemon is not running" >&2; exit 2; }
[[ -n "$awww_pid" ]] || { echo "error: awww-daemon is not running" >&2; exit 2; }

# Put both daemons into the same settled state before sampling. Wallr may
# restore a cached wallpaper during startup; awww is started with --no-cache,
# so sampling before sending it the same image would compare an active
# compositor buffer with an empty daemon rather than equivalent workloads.
"$wallr_bin" set "${images[0]}" --duration 0 --no-theme >/dev/null 2>&1 || true
awww img "${images[0]}" >/dev/null 2>&1 || true
sleep 2

sample_process() {
    local label=$1 pid=$2
    ps -o pid=,rss=,vsz=,%cpu=,nlwp=,etime=,comm= -p "$pid" \
        | awk -v label="$label" '{$1=$1; print label, $0}'
}

process_row() {
    local label=$1 pid=$2
    local sample
    if sample=$(ps -o rss=,%cpu=,nlwp= -p "$pid" 2>/dev/null) && [[ -n "$sample" ]]; then
        awk -v label="$label" '{printf "| %s | %.1f | %s%% | %s |\n", label, $1 / 1024, $2, $3}' <<<"$sample"
    else
        echo "| $label | unavailable | unavailable | unavailable |"
    fi
}

sample_memory() {
    local label=$1 pid=$2
    if [[ ! -r "/proc/$pid/smaps_rollup" ]]; then
        echo "| $label | unavailable | unavailable | unavailable | unavailable | unavailable |"
        return 0
    fi
    awk -v label="$label" '
        /^Rss:/ { rss=$2 }
        /^Pss:/ { pss=$2 }
        /^Private_Dirty:/ { dirty=$2 }
        /^Private_Clean:/ { clean=$2 }
        /^Anonymous:/ { anon=$2 }
        END {
            printf "| %s | %.1f | %.1f | %.1f | %.1f | %.1f |\n",
                label, rss / 1024, pss / 1024, anon / 1024,
                (dirty + clean) / 1024, (rss - anon) / 1024
        }
    ' "/proc/$pid/smaps_rollup"
}

print_memory_table() {
    echo "| Program | RSS (MiB) | PSS (MiB) | Anonymous (MiB) | Private (MiB) | Non-anonymous RSS (MiB) |"
    echo "|:--|--:|--:|--:|--:|--:|"
    sample_memory Wallr "$wallr_pid"
    sample_memory awww "$awww_pid"
}

wallr_round_wins=0
awww_round_wins=0
tied_rounds=0

# Parse a hyperfine --export-markdown table and print a one-line 🏆 winner.
# Expects two data rows (Wallr vs awww) with a Mean column sharing one unit.
# Prints the winner line and updates the running win counters.
announce_latency_winner() {
    local table=$1
    local wallr_mean="" awww_mean="" unit="ms"

    unit=$(sed -n 's/.*Mean \[\([^]]*\)\].*/\1/p' "$table" | head -n 1)
    [[ -n "$unit" ]] || unit="ms"

    # Hyperfine's markdown exporter keeps commands in the second pipe field
    # and the mean in the third. Parse those fields directly; the previous
    # line-by-line shell matcher could mistake the alignment row for a data
    # row and silently discard otherwise valid measurements.
    wallr_mean=$(awk -F'|' '$2 ~ /wallr/ {print $3; exit}' "$table" | awk '{print $1}')
    awww_mean=$(awk -F'|' '$2 ~ /awww/ {print $3; exit}' "$table" | awk '{print $1}')

    if [[ -z "$wallr_mean" || -z "$awww_mean" ]]; then
        echo "> ⚠️ Winner: could not be determined from this run."
        return 0
    fi

    local faster ratio
    faster=$(awk -v w="$wallr_mean" -v a="$awww_mean" 'BEGIN { print (w <= a) ? "wallr" : "awww" }')
    ratio=$(awk -v w="$wallr_mean" -v a="$awww_mean" 'BEGIN { lo = (w < a ? w : a); hi = (w > a ? w : a); printf "%.2f", hi / lo }')

    if [[ "$wallr_mean" == "$awww_mean" ]]; then
        tied_rounds=$((tied_rounds + 1))
        echo "> 🤝 Winner: tie; Wallr and awww both averaged ${wallr_mean} ${unit}."
    elif [[ "$faster" == "wallr" ]]; then
        wallr_round_wins=$((wallr_round_wins + 1))
        echo "> 🏆 Winner: Wallr: ${ratio}× faster than awww (${wallr_mean} ${unit} vs ${awww_mean} ${unit})."
    else
        awww_round_wins=$((awww_round_wins + 1))
        echo "> 🏆 Winner: awww: ${ratio}× faster than Wallr (${awww_mean} ${unit} vs ${wallr_mean} ${unit})."
    fi
}

# Compare current RSS of both daemons and print a one-line 🏆 winner.
# Args: human-readable context label (e.g. "idle baseline")
announce_resource_winner() {
    local context=$1
    local wallr_sample awww_sample wallr_rss awww_rss wallr_cpu awww_cpu
    wallr_sample=$(ps -o rss=,%cpu= -p "$wallr_pid" 2>/dev/null || true)
    awww_sample=$(ps -o rss=,%cpu= -p "$awww_pid" 2>/dev/null || true)
    wallr_rss=$(awk '{print $1}' <<<"$wallr_sample")
    wallr_cpu=$(awk '{print $2}' <<<"$wallr_sample")
    awww_rss=$(awk '{print $1}' <<<"$awww_sample")
    awww_cpu=$(awk '{print $2}' <<<"$awww_sample")

    if [[ ! "$wallr_rss" =~ ^[0-9]+$ || ! "$awww_rss" =~ ^[0-9]+$ ]]; then
        echo "> ⚠️ Winner ($context RAM): could not be determined."
        return 0
    fi

    local wallr_mib awww_mib ratio
    wallr_mib=$(awk -v rss="$wallr_rss" 'BEGIN { printf "%.1f", rss / 1024 }')
    awww_mib=$(awk -v rss="$awww_rss" 'BEGIN { printf "%.1f", rss / 1024 }')
    ratio=$(awk -v w="$wallr_rss" -v a="$awww_rss" 'BEGIN { lo = (w < a ? w : a); hi = (w > a ? w : a); if (lo == 0) print "n/a"; else printf "%.2f", hi / lo }')
    [[ -n "$wallr_cpu" ]] || wallr_cpu="n/a"
    [[ -n "$awww_cpu" ]] || awww_cpu="n/a"

    if ((wallr_rss == awww_rss)); then
        tied_rounds=$((tied_rounds + 1))
        echo "> 🤝 Winner ($context RAM): tie; both use ${wallr_mib} MiB."
    elif ((wallr_rss < awww_rss)); then
        wallr_round_wins=$((wallr_round_wins + 1))
        echo "> 🏆 Winner ($context RAM): Wallr: ${ratio}× leaner than awww (${wallr_mib} MiB vs ${awww_mib} MiB)."
    else
        awww_round_wins=$((awww_round_wins + 1))
        echo "> 🏆 Winner ($context RAM): awww: ${ratio}× leaner than Wallr (${awww_mib} MiB vs ${wallr_mib} MiB)."
    fi
    echo "> ℹ️ CPU snapshot ($context): Wallr ${wallr_cpu}% vs awww ${awww_cpu}% (instant sample, noisy)."
}

run_table() {
    local warmup=$1 runs=$2
    shift 2
    local table
    table=$(mktemp)
    timeout --foreground 90s hyperfine --warmup "$warmup" --runs "$runs" \
        --export-markdown "$table" "$@" >/dev/null 2>&1
    sed "s|$wallr_bin|wallr|g" "$table"
    echo
    announce_latency_winner "$table"
    rm -f "$table"
}

echo "## 📊 Daemon baseline"
echo
echo "| Program | RAM (MiB) | CPU | Threads |"
echo "|:--|--:|--:|--:|"
process_row Wallr "$wallr_pid"
process_row awww "$awww_pid"
echo
print_memory_table
echo
announce_resource_winner "idle baseline"

echo
echo "## ⚡ Hot same-image static latency"
echo
echo "This measures repeated static requests with transitions disabled."
echo "🏆 The fastest mean latency wins this round."
echo
run_table 2 20 \
    "$wallr_bin set '${images[0]}' --duration 0 --no-theme" \
    "awww img '${images[0]}'"

echo
echo "## 🚀 Zero-duration static path"
echo
echo 'This measures a static wallpaper with no transition animation.'
echo '🏆 The fastest mean latency wins this round.'
echo 'Wallr uses `wl_shm` when the output has not used the GPU presentation path; otherwise it preserves the GPU surface protocol for compositor compatibility.'
echo
run_table 2 20 \
    "$wallr_bin set '${images[0]}' --duration 0 --no-theme" \
    "awww img '${images[0]}'"

echo
echo "## 🔄 Distinct-image switch latency"
echo
echo "This measures changing to several different images."
echo "🏆 Each image below is its own round with its own winner."
echo
for image in "${images[@]}"; do
    echo "### 🖼️ $(basename "$image")"
    run_table 1 5 \
        "$wallr_bin set '$image' --duration 0 --no-theme" \
        "awww img '$image'"
done

if [[ "${BENCHMARK_INCLUDE_ANIMATED:-1}" != 0 && -s "$root_dir/samples/extra.gif" ]]; then
    echo
    echo "## 🎞️ Animated GIF submission latency"
    echo
    echo "Both programs support GIF input. This measures submission latency only;"
    echo "smoothness and steady-state animation cost require a separate profiler."
    echo "🏆 The fastest mean submission wins this round."
    echo
    run_table 1 5 \
        "$wallr_bin set '$root_dir/samples/extra.gif' --duration 0 --no-theme" \
        "awww img '$root_dir/samples/extra.gif'"
fi

# End any live animated workload before taking the resident-state sample.
# Otherwise the report measures the decoder, staging buffers, and live player
# by design rather than settled daemon idle state.
"$wallr_bin" set "${images[0]}" --duration 0 --no-theme >/dev/null 2>&1 || true
awww img "${images[0]}" >/dev/null 2>&1 || true
sleep 3
echo
echo "## 🧘 Post-workload resident state"
echo
echo "This shows memory and CPU after all wallpaper changes finish."
echo
echo "| Program | RAM (MiB) | CPU | Threads |"
echo "|:--|--:|--:|--:|"
process_row Wallr "$wallr_pid"
process_row awww "$awww_pid"
echo
print_memory_table
echo
announce_resource_winner "post-workload"
echo
echo "## 🩺 Daemon health"
echo
echo "| Check | Result |"
echo "|:--|:--|"
if kill -0 "$wallr_pid" 2>/dev/null; then
    echo "| Wallr release daemon after workload | running |"
else
    echo "| Wallr release daemon after workload | stopped unexpectedly |"
fi
if [[ -n "${local_log:-}" && -s "$local_log" ]]; then
    echo
    echo "Wallr daemon log (last 40 lines):"
    echo
    echo '```text'
    tail -n 40 "$local_log"
    echo '```'
fi
echo
echo "## 🏁 Production verdict"
echo
if ((wallr_round_wins > awww_round_wins)); then
    echo "> 🏆 Overall winner: Wallr: ${wallr_round_wins} round(s) won vs ${awww_round_wins} for awww (${tied_rounds} tied)."
elif ((awww_round_wins > wallr_round_wins)); then
    echo "> 🏆 Overall winner: awww: ${awww_round_wins} round(s) won vs ${wallr_round_wins} for Wallr (${tied_rounds} tied)."
elif (((wallr_round_wins + awww_round_wins + tied_rounds) > 0)); then
    echo "> 🤝 Overall result: tie: ${wallr_round_wins} round(s) each (${tied_rounds} tied)."
else
    echo "> ⚠️ Overall winner: could not be determined."
fi
echo
echo "Wallr and awww are optimized for different trade-offs. This run measures"
echo "Wallr's switching latency against awww's switching latency. It does not"
echo "establish a universal winner across all GPUs, compositors, resolutions,"
echo "idle states, animated images, or video formats."
echo
echo "Use the switching tables for latency decisions and the resident-state"
echo "table for idle RAM/CPU decisions."
echo
echo "Measured scope: this run reports the measured winner for each workload."
echo "Static switching, idle resource use, and duplicate requests are not"
echo "universal claims; repeat the benchmark on the target compositor and GPU."
echo
printf 'Report saved to: `%s`\n' "$report_file"
