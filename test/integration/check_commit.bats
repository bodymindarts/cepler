#!/usr/bin/env bats

load "helpers"

setup_file() {
  echo "Preparing 'check_commit'"
  prepare_test "check_commit"
}

teardown_file() {
  echo "Tearing down 'check_commit'"
  reset_repo_state
}

@test "Reports trigger commit" {
  add_commit="3997b50"
  cmd check -e testflight | grep "${add_commit}"
  cmd record -e testflight
  cmd check -e staging | grep "${add_commit}"
}
