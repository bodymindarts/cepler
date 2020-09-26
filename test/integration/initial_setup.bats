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

@test "Accepts uncommitted cepler.yml" {
  cat <<EOF > `fixture`/cepler.yml
environments:
  testflight:
    latest:
    - file.yml
EOF

  cmd check -e testflight
}
