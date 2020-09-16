#!/bin/bash

set -eux

export CARGO_HOME="$(pwd)/cargo-home"
export CARGO_TARGET_DIR="$(pwd)/cargo-target-dir"

export PATH="${PATH}:${CARGO_TARGET_DIR}/debug/"

pushd repo

git config --global user.email "ceplerbot@misthos.io"
git config --global user.name "Ci Bot"

make integration
