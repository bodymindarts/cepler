#!/bin/bash

set -eu

VERSION=""
if [[ -f version/number ]];then
  VERSION="$(cat version/number)"
fi

REPO=${REPO:-repo}
BINARY=cepler
OUT=${OUT:-none}
WORKSPACE="$(pwd)"
export CARGO_HOME="$(pwd)/cargo-home"
export CARGO_TARGET_DIR="$(pwd)/cargo-target-dir"

pushd ${REPO}

REPO=$(pwd) make build-${TARGET}-release

set -x
cd ${CARGO_TARGET_DIR}/release
OUT_DIR="${BINARY}-${TARGET}-${VERSION}"
mkdir "${OUT_DIR}"
mv ./${BINARY} ${OUT_DIR}
tar -czvf ${OUT_DIR}.tar.gz ${OUT_DIR}

mv ${OUT_DIR}.tar.gz ${WORKSPACE}/${OUT}/
