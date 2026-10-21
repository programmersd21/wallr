#!/usr/bin/env bash
set -euo pipefail

root_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
vcpkg_commit=45f9f39362a4c52e2b1fbe57b7e649db7f3d96d4
cache_home=${XDG_CACHE_HOME:-"$HOME/.cache"}
vcpkg_root="$cache_home/wallr/vcpkg"

usage() {
    cat <<'USAGE'
Usage: scripts/install-static-ffmpeg.sh [--prefix DIR] [--help]

Build Wallr's static FFmpeg toolchain with the same pinned vcpkg revision and
features as the Linux release workflow. Installs build files under the user
cache; it does not install FFmpeg system-wide.

Options:
  --prefix DIR  vcpkg root (default: $XDG_CACHE_HOME/wallr/vcpkg)
  -h, --help    show this help
USAGE
}

while (($#)); do
    case "$1" in
        --prefix)
            (($# >= 2)) || { echo "error: --prefix needs a directory" >&2; exit 2; }
            vcpkg_root=$2
            shift 2
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "error: unknown option: $1" >&2
            usage >&2
            exit 2
            ;;
    esac
done

if [[ $(uname -s) != Linux || $(uname -m) != x86_64 ]]; then
    echo "error: static FFmpeg setup currently supports x86_64 Linux only" >&2
    exit 2
fi

missing=()
for command_name in git curl zip unzip tar cmake ninja pkg-config make cc clang; do
    command -v "$command_name" >/dev/null || missing+=("$command_name")
done
if ! command -v nasm >/dev/null && ! command -v yasm >/dev/null; then
    missing+=(nasm-or-yasm)
fi
if ((${#missing[@]})); then
    echo "error: install these build tools first: ${missing[*]}" >&2
    echo "Arch/CachyOS: sudo pacman -S --needed base-devel clang git curl zip unzip cmake ninja pkgconf nasm" >&2
    echo "Debian/Ubuntu: sudo apt install build-essential clang libclang-dev git curl zip unzip cmake ninja-build pkg-config nasm" >&2
    echo "Fedora: sudo dnf install @development-tools clang-devel git curl zip unzip cmake ninja-build pkgconf-pkg-config nasm" >&2
    exit 2
fi

mkdir -p "$(dirname "$vcpkg_root")"
if [[ -e "$vcpkg_root" && ! -d "$vcpkg_root/.git" ]]; then
    echo "error: refusing to use a non-vcpkg directory: $vcpkg_root" >&2
    exit 2
fi
if [[ -d "$vcpkg_root/.git" ]]; then
    existing_commit=$(git -C "$vcpkg_root" rev-parse HEAD)
    if [[ "$existing_commit" != "$vcpkg_commit" ]]; then
        echo "error: $vcpkg_root is at $existing_commit, expected pinned vcpkg $vcpkg_commit" >&2
        echo "choose another directory with --prefix" >&2
        exit 2
    fi
else
    git clone --depth 1 --branch master https://github.com/microsoft/vcpkg "$vcpkg_root"
    git -C "$vcpkg_root" fetch --depth 1 origin "$vcpkg_commit"
    git -C "$vcpkg_root" checkout --detach "$vcpkg_commit"
fi

if [[ ! -x "$vcpkg_root/vcpkg" ]]; then
    "$vcpkg_root/bootstrap-vcpkg.sh" -disableMetrics
fi

"$vcpkg_root/vcpkg" install \
    'ffmpeg[core,avcodec,avformat,swscale,swresample,nvcodec]:x64-linux-static' \
    --overlay-triplets="$root_dir/.github/vcpkg-triplets"

media_prefix="$vcpkg_root/installed/x64-linux-static"
pkg_config_path="$media_prefix/lib/pkgconfig"
printf '\nstatic FFmpeg is ready. Set the build environment and compile with:\n\n'
printf 'export MEDIA_PREFIX=%q\nexport PKG_CONFIG_PATH=%q\nexport PKG_CONFIG_ALL_STATIC=1\n' \
    "$media_prefix" "$pkg_config_path"
printf 'cargo build --release --bins --locked --features static-ffmpeg\n'
printf '\nFFmpeg libraries: %s\n' "$media_prefix/lib"
