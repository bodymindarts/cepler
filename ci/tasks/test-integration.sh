#!/bin/bash

set -eux

export CARGO_HOME="$(pwd)/cargo-home"
export CARGO_TARGET_DIR="$(pwd)/cargo-target-dir"

export PATH="${PATH}:${CARGO_TARGET_DIR}/debug/"

pushd repo

git config --global user.email "bot@cepler.dev"
git config --global user.name "CI Bot"

make integration
