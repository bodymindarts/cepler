REPO_ROOT=$(git rev-parse --show-toplevel)

cmd() {
  cepler=${REPO_ROOT}/target/debug/cepler
  if [[ ! -z ${CARGO_TARGET_DIR} ]] ; then
    cepler=${CARGO_TARGET_DIR}/debug/cepler
  fi

  echo "${cepler} -c test/fixtures/$(basename ${BATS_TEST_FILENAME%%.*})/cepler.yml $@"

  ${cepler} -c test/fixtures/$(basename ${BATS_TEST_FILENAME%%.*})/cepler.yml $@
}

config() {
  echo "test/fixtures/$(basename ${BATS_TEST_FILENAME%%.*})/cepler.yml"
}

state() {
  echo "test/fixtures/$(basename ${BATS_TEST_FILENAME%%.*})/.cepler/$1.state"
}

fixture() {
  echo "test/fixtures/$(basename ${BATS_TEST_FILENAME%%.*})"
}

cache_value() {
  echo $2 > ${BATS_TMPDIR}/$1
}

read_value() {
  cat ${BATS_TMPDIR}/$1
}

prepare_test() {
  cache_value "head_ref" $(git rev-parse --short HEAD)
  cache_value "current_branch" $(git branch --show-current)
  git add ${REPO_ROOT}
  git commit -m 'Caching uncommited state' || true
  git branch -D $1 || true
  git checkout -b $1
}

reset_repo_state() {
  git reset --hard
  git clean -f
  git checkout $(read_value "current_branch")
  git reset $(read_value "head_ref")
}
