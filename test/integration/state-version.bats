#!/usr/bin/env bats

load "helpers"

setup_file() {
  echo "Preparing 'state-version'"
  prepare_test "state-version"
}

teardown_file() {
  echo "Tearing down 'state-version'"
  reset_repo_state
}

@test "Persists version" {
  cmd check -e testflight
  cmd record -e testflight
  grep 'version: 1' $(state "testflight")
}

@test "Bumps version" {
  echo "file_new: {}" > `fixture`/file.yml
  git commit -am 'Update file.yml'
  cmd check -e testflight
  cmd record -e testflight
  grep 'version: 2' $(state "testflight")
}

