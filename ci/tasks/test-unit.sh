#!/bin/bash

set -eux

export CARGO_HOME="$(pwd)/cargo-home"
export CARGO_TARGET_DIR="$(pwd)/cargo-target-dir"

pushd repo

make test-in-ci
