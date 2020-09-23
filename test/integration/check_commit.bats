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

@test "Ignore unrelated commits" {
  add_commit="3997b50"

  echo "bla" > `fixture`/bla.yml
  git add .
  git commit -m 'Add bla.yml'
  cmd check -e testflight | grep "${add_commit}"
}

@test "Reports trigger commit" {
  add_commit="3997b50"
  cmd check -e testflight | grep "${add_commit}"
  cmd record -e testflight
  cmd check -e staging | grep "${add_commit}"
}

@test "Reports delete" {
  add_commit="3997b50"

  rm `fixture`/file.yml
  git commit -am 'Remove file.yml'
  head=$(git rev-parse --short HEAD)
  cmd check -e testflight | grep "${head}"
  cmd check -e staging | grep "${add_commit}"
}

@test "Reports original on downstream" {
  add_commit="3997b50"

  echo 'new_flile: {}' > `fixture`/file.yml
  git add .
  git commit -m 'Re-add file.yml'
  head=$(git rev-parse --short HEAD)
  cmd check -e testflight | grep "${head}"
  cmd check -e staging | grep "${add_commit}"
}
