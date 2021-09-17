#!/usr/bin/env bats

load "helpers"

setup_file() {
  echo "Preparing 'gates'"
  prepare_test "gates"
}

teardown_file() {
  echo "Tearing down 'gates'"
  reset_repo_state
}

@test "Fails when gate isn't specified" {
  run cmd -g `fixture`/cepler-gates.yml check -e missing
  [ "$status" -eq 1 ]
}

@test "Takes latest when its HEAD" {
  add_commit="aef8e29"

  cmd -g `fixture`/cepler-gates.yml check -e head | grep "${add_commit}"

  echo "new: {}" > `fixture`/latest.yml
  git commit -am 'Update latest.yml'
  head=$(git rev-parse --short HEAD)

  cmd -g `fixture`/cepler-gates.yml check -e head | grep "${head}"
}

@test "Only searches till gate" {
  last_changed="aef8e29"

  cmd -g `fixture`/cepler-gates.yml check -e gated | grep "${last_changed}"
  cmd -g `fixture`/cepler-gates.yml record -e gated

  echo "new: {}" > `fixture`/gated.yml
  git commit -am 'Update gated.yml'
  head=$(git rev-parse --short HEAD)

  run cmd -g `fixture`/cepler-gates.yml check -e gated
  [ "$status" -eq 2 ]

  cmd check -e gated | grep "${head}"
}

@test "Only prepares up to gate" {
  cmd prepare -e gated
  grep new `fixture`/gated.yml

  cmd -g `fixture`/cepler-gates.yml prepare -e gated
  grep gated `fixture`/gated.yml
}
