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

@test "Prepare staging shouldn't delete cepler.yml" {
  cmd prepare -e staging

  [ -f $(config) ]

  git checkout .
}

@test "Head files override propagated_files" {
  echo "staging_new: {}" > $(fixture)/staging.yml
  git commit -am 'Update staging.yml'

  cmd prepare -e staging
  run grep 'staging_new' $(fixture)/staging.yml

  [ "$status" -eq 0 ]

  git checkout .
}
