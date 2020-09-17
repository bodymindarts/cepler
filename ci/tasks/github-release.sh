#!/bin/bash

set -eu

VERSION=$(cat version/number)
BINARY=cepler
WORKSPACE="$( cd "$( dirname "${BASH_SOURCE[0]}" )/../../../" >/dev/null && pwd )"
REPO_ROOT="${WORKSPACE}/prepared-repo/git"
RELEASE_ROOT="${WORKSPACE}/gh-release"
RELEASE_NAME="cepler release"

mv ${REPO_ROOT}/../notes.md ${RELEASE_ROOT}/notes.md

mkdir -p ${RELEASE_ROOT}/artifacts
mv x86_64-unknown-linux-musl/* ${RELEASE_ROOT}/artifacts/

echo "v${VERSION}"                         > ${RELEASE_ROOT}/tag
echo "${RELEASE_NAME} v${VERSION}"         > ${RELEASE_ROOT}/name
