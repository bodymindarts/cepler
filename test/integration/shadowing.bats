#!/usr/bin/env bats

load "helpers"

setup_file() {
  echo "Preparing 'shadowing'"
  prepare_test "shadowing"
}

teardown_file() {
  echo "Tearing down 'shadowing'"
  reset_repo_state
}

@test "Testflight needs deploying" {
  cmd check -e testflight
}

@test "Staging doesn't need deploying" {
  run cmd check -e staging
  [ "$status" -eq 1 ]
}

@test "Record testflight should ignore cepler.yml" {
  cmd record -e testflight

  files=$(grep '.yml' $(state "testflight") | wc -l)
  [ "$files" -eq 3 ]

  run grep cepler.yml $(state "testflight")
  [ "$status" -ne 0 ]
}
