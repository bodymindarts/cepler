#!/usr/bin/env bats

load "helpers"

setup_file() {
  echo "Preparing 'glob'"
  prepare_test "glob"
}

teardown_file() {
  echo "Tearing down 'glob'"
  reset_repo_state
}

@test "ls testflight finds 3 files" {
  files="$(cmd ls -e testflight | wc -l)"
  [ "${files}" -eq 4 ]
}

@test "ls subdir finds 4 files" {
  files="$(cmd ls -e subdir | wc -l)"
  [ "${files}" -eq 5 ]
}

@test "Prepare staging should render 3 files" {
  cmd record -e testflight
  cmd prepare -e staging --force-clean
  files=$(find . -name '*.yml' | wc -l)
  [ "${files}" -eq 4 ]
}
