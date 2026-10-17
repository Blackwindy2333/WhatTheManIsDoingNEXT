#!/usr/bin/env bash
# 构建 Rust JNI 桥 -> app/src/main/jniLibs/<abi>/libwtmd_android_bridge.so
# 用法：bash Client/android/rust/build-native.sh
# 依赖：cargo-ndk、NDK 27.x（ANDROID_HOME 或 ANDROID_NDK_HOME 下）、
#       rustup 目标 aarch64-linux-android / armv7-linux-androideabi / x86_64-linux-android。
set -euo pipefail

RUST_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT_DIR="$RUST_DIR/../app/src/main/jniLibs"
mkdir -p "$OUT_DIR"

cd "$RUST_DIR"
cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 -o "$OUT_DIR" build --release

echo "OK: jniLibs written to $OUT_DIR"
find "$OUT_DIR" -name '*.so'
