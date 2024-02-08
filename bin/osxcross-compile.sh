#!/bin/bash

MACOS_TARGET="x86_64-apple-darwin"

echo "Building target for platform ${MACOS_TARGET}"
echo

# Make libz-sys (git2-rs -> libgit2-sys -> libz-sys) build as a statically linked lib
# This prevents the host zlib from being linked
export LIBZ_SYS_STATIC=1

# Use Clang for C/C++ builds
export CC=o64-clang
export CXX=o64-clang++

echo 'targets = ["x86_64-apple-darwin"]' >> rust-toolchain.toml

cargo build --release --target "${MACOS_TARGET}" --all-features

echo
echo Done
