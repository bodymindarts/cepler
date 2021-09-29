#!/usr/bin/env bats

load "helpers"

setup_file() {
  echo "Preparing 'propagation'"
  prepare_test "propagation"
}

teardown_file() {
  echo "Tearing down 'propagation'"
  reset_repo_state
}

@test "No propagation queue when changing non-relevant file" {
  cmd check -e testflight
  cmd record -e testflight
  cmd check -e staging
  cmd record -e staging

  echo "testflight_new: {}" > `fixture`/testflight.yml
  git commit -am 'Update testflight.yml'

  cmd check -e testflight
  cmd record -e testflight

  run cmd check -e staging
  [ "$status" -eq 2 ]

  echo "propagated_new: {}" > `fixture`/propagated.yml
  git commit -am 'Update propagated.yml'

  cmd check -e testflight
  cmd record -e testflight

  cmd check -e staging
}

@test "Prepares next in queue" {
  echo "propagated_other: {}" > `fixture`/propagated.yml
  git commit -am 'Update propagated.yml again'

  cmd check -e testflight
  cmd record -e testflight

  cmd prepare -e staging
  grep 'propagated_new' `fixture`/propagated.yml
}

@test "Can ignore queue" {
  cmd --ignore-queue prepare -e staging
  grep 'propagated_other' `fixture`/propagated.yml
}
