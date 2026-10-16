#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "$0")/.." && pwd)"
cd "$project_dir"

: "${ANDROID_HOME:=${ANDROID_SDK_ROOT:-$HOME/Library/Android/sdk}}"
: "${ANDROID_NDK_HOME:=${ANDROID_NDK_ROOT:-$ANDROID_HOME/ndk/27.2.12479018}}"
: "${CMAKE:=$ANDROID_HOME/cmake/3.22.1/bin/cmake}"
: "${SDL2_TOOLCHAIN:=$project_dir/android/cmake/ikura-android.toolchain.cmake}"
export ANDROID_HOME ANDROID_NDK_HOME CMAKE SDL2_TOOLCHAIN

linker="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/darwin-x86_64/bin/aarch64-linux-android24-clang"
if [[ "$(uname -s)" == Linux ]]; then
    linker="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin/aarch64-linux-android24-clang"
fi
if [[ ! -x "$linker" ]]; then
    echo "Android linker not found: $linker" >&2
    exit 1
fi
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$linker"

cargo build --release --target aarch64-linux-android --lib
sdl2_lib="$(find target/aarch64-linux-android/release/build -path '*/out/lib/libSDL2.so' -print -quit)"
mixer_lib="$(find target/aarch64-linux-android/release/build -path '*/out/sdl2-mixer-install/lib/libSDL2_mixer.so' -print -quit)"
if [[ -z "$sdl2_lib" || -z "$mixer_lib" ]]; then
    echo "SDL2 native libraries were not produced by the Android build" >&2
    exit 1
fi

jni="$project_dir/android/app/src/main/jniLibs/arm64-v8a"
mkdir -p "$jni"
cp "$sdl2_lib" "$jni/libSDL2.so"
cp "$mixer_lib" "$jni/libSDL2_mixer.so"
cp target/aarch64-linux-android/release/libikura_rust_prototype.so "$jni/libmain.so"
