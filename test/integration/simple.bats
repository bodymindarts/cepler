#!/usr/bin/env bats

load "helpers"

setup_file() {
  echo "Preparing 'simple'"
  prepare_test "simple"
}

teardown_file() {
  echo "Tearing down 'simple'"
  reset_repo_state
}

@test "Testflight needs deploying" {
  cmd check -e testflight
}

@test "Staging doesn't need deploying" {
  run cmd check -e staging
  [ "$status" -eq 1 ]
}

@test "Record testflight should hash file" {
  file_hash=$(git hash-object `fixture`/file.yml)
  cmd record -e testflight

  grep ${file_hash} $(state "testflight")
  n_files=$(grep 'file_hash:' $(state "testflight") | wc -l)
  [ "$n_files" -eq 1 ]
}

@test "Prepare staging should reset file" {
  echo "file_new: {}" > `fixture`/file.yml
  git commit -am 'Update file.yml'

  cmd prepare -e staging
  file_hash=$(git hash-object `fixture`/file.yml)
  grep ${file_hash} $(state "testflight")
  cmd record -e staging
  grep ${file_hash} $(state "staging")
  n_files=$(grep 'file_hash:' $(state "staging") | wc -l)
  [ "$n_files" -eq 2 ]

  git checkout .
}

@test "Recording multiple states should add propagation_queue" {
  cmd check -e testflight
  cmd record -e testflight
  run grep 'propagation_queue' $(state "testflight")
  [ "$status" -eq 1 ]
  echo "file_new_1: {}" > `fixture`/file.yml
  git commit -am 'Update file.yml 1'
  cmd check -e testflight
  cmd record -e testflight
  grep 'propagation_queue' $(state "testflight")
}

@test "Prepare + record should step through queue" {
  cmd check -e staging
  cmd prepare -e staging
  grep 'file_new' `fixture`/file.yml
  cmd record -e staging
  cmd check -e staging
  cmd prepare -e staging
  grep 'file_new_1' `fixture`/file.yml
  cmd record -e staging
  run cmd check -e staging
  [ "$status" -eq 2 ]
}

@test "Reproduce should reset file" {
  echo "file_reproduce: {}" > `fixture`/file.yml
  git commit -am 'Update file.yml for reproduction'
  cmd check -e testflight
  cmd record -e testflight

  cmd reproduce -e staging
  cmd prepare -e staging
  grep 'file_new' `fixture`/file.yml
}
