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
  add_commit="fde0360"

  echo "bla" > `fixture`/bla.yml
  git add .
  git commit -m 'Add bla.yml'
  cmd check -e testflight | grep "${add_commit}"
}

@test "Reports trigger commit" {
  add_commit="fde0360"
  cmd check -e testflight | grep "${add_commit}"
  cmd record -e testflight
  head=$(git rev-parse --short HEAD)
  cmd check -e staging | grep "${head}"
}

@test "Reports delete" {
  rm `fixture`/file.yml
  git commit -am 'Remove file.yml'
  head=$(git rev-parse --short HEAD)
  cmd check -e testflight | grep "${head}"
  record_commit=$(git log -n 1 --pretty=format:%h -- $(state "testflight"))
  cmd check -e staging | grep "${record_commit}"
}

@test "Reports original on downstream" {
  echo 'new_flile: {}' > `fixture`/file.yml
  git add .
  git commit -m 'Re-add file.yml'
  head=$(git rev-parse --short HEAD)
  cmd check -e testflight | grep "${head}"
  record_commit=$(git log -n 1 --pretty=format:%h -- $(state "testflight"))
  cmd check -e staging | grep "${record_commit}"
}
