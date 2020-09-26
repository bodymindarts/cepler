#!/usr/bin/env bats

load "helpers"

setup_file() {
  echo "Preparing 'initial_setup'"
  prepare_test "initial_setup"
}

teardown_file() {
  echo "Tearing down 'initial_setup'"
  reset_repo_state
}

@test "Write cepler.yml" {
  cmd check -e testflight
}
