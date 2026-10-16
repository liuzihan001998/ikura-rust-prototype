#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "$0")/.." && pwd)"
original_dir="${1:-$(cd "$project_dir/.." && pwd)}"
extracted_dir="${2:-/Users/a1/Desktop/dev/ikura/game_data}"
archive="$project_dir/android/app/src/main/assets/ikura-data.zip"
stage_dir="$(mktemp -d "${TMPDIR:-/tmp}/ikura-android-assets.XXXXXX")"
trap 'rm -rf "$stage_dir"' EXIT

for path in "$original_dir/DATA" "$extracted_dir/PNG-images" "$extracted_dir/OGG-audio" "$project_dir/data/scripts.json" "$project_dir/data/zh-CN.json"; do
    if [[ ! -e "$path" ]]; then
        echo "Required game resource not found: $path" >&2
        exit 1
    fi
done

root="$stage_dir/game_data"
mkdir -p "$root"
cp -R "$extracted_dir/PNG-images" "$root/"
cp -R "$extracted_dir/OGG-audio" "$root/"
cp "$original_dir/DATA" "$root/DATA"
cp "$project_dir/data/scripts.json" "$root/scripts.json"
cp "$project_dir/data/zh-CN.json" "$root/zh-CN.json"

mkdir -p "$(dirname "$archive")"
rm -f "$archive"
(cd "$stage_dir" && /usr/bin/zip -q -r -0 "$archive" game_data)
printf 'Staged Android game data: %s (%s)\n' "$archive" "$(du -h "$archive" | awk '{print $1}')"
