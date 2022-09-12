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

  echo "new: {}" > `fixture`/gated.yml
  git commit -am 'Update gated.yml'
  head=$(git rev-parse --short HEAD)

  cmd -g `fixture`/cepler-gates.yml check -e gated | grep "${last_changed}"
  cmd check -e gated | grep "${head}"
}

@test "Only prepares up to gate" {
  cmd prepare -e gated
  grep new `fixture`/gated.yml

  cmd -g `fixture`/cepler-gates.yml prepare -e gated
  grep gated `fixture`/gated.yml

  cmd prepare -e gated
  grep new `fixture`/gated.yml
}

@test "Records as gated" {
  cmd -g `fixture`/cepler-gates.yml prepare -e gated
  before=$(git rev-parse --short HEAD)
  cmd record -e gated
  grep dirty `state gated`

  git reset --hard "${before}"

  cmd -g `fixture`/cepler-gates.yml prepare -e gated
  cmd -g `fixture`/cepler-gates.yml record -e gated
  run grep dirty `state gated`
  [ "$status" -eq 1 ]

  run cmd -g `fixture`/cepler-gates.yml check -e gated
  [ "$status" -eq 2 ]
}

@test "Correctly triggers with queue and supports rollback" {
  cmd -g `fixture`/cepler-gates.yml record -e queued
  before=$(git rev-parse HEAD)

  cat <<EOF > `fixture`/cepler-gates.yml
queued: HEAD
propagated: ${before}
EOF

  cmd -g `fixture`/cepler-gates.yml check -e propagated
  cmd -g `fixture`/cepler-gates.yml record -e propagated

  echo "trigger1: {}" > `fixture`/queued.yml
  git commit -am 'Update queued.yml'
  cmd -g `fixture`/cepler-gates.yml record -e queued
  trigger1=$(git rev-parse HEAD)
  echo "trigger2: {}" > `fixture`/queued.yml
  git commit -am 'Update queued.yml again'
  cmd -g `fixture`/cepler-gates.yml record -e queued
  trigger2=$(git rev-parse HEAD)

  run cmd -g `fixture`/cepler-gates.yml check -e propagated
  [ "$status" -eq 2 ]

  cat <<EOF > `fixture`/cepler-gates.yml
queued: HEAD
propagated: ${trigger2}
EOF

  cmd -g `fixture`/cepler-gates.yml check -e propagated | grep "${trigger1}"
  cmd -g `fixture`/cepler-gates.yml prepare -e propagated
  cmd -g `fixture`/cepler-gates.yml record -e propagated

  cmd -g `fixture`/cepler-gates.yml check -e propagated | grep "${trigger2}"
  cmd -g `fixture`/cepler-gates.yml prepare -e propagated
  cmd -g `fixture`/cepler-gates.yml record -e propagated

  run cmd -g `fixture`/cepler-gates.yml check -e propagated
  [ "$status" -eq 2 ]

  cat <<EOF > `fixture`/cepler-gates.yml
queued: HEAD
propagated: ${trigger1}
EOF

  cmd -g `fixture`/cepler-gates.yml check -e propagated | grep "${trigger1}"
  cmd -g `fixture`/cepler-gates.yml prepare -e propagated
  grep "trigger1" `fixture`/queued.yml
}
