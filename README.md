# CEPler

Environment Propagator is here to help you manage state of files representing a system deployed to multiple environments.

## Installation

If you have a rust toolchain you can:
```
cargo install cepler
```

Otherwise pre-compiled binaries are attached to the [github releases](https://github.com/bodymindarts/cepler/releases) and can be downloaded and unpacked.

## Usage

To use cepler first you must write a config file (by default expected at `cepler.yml` but can be overriden via `-c` flag or `CONFIG_FILE` env var).
The config file specifies:
- What environments there are
- What files are relevant to a specific environment
- How the config files are propagated across the environments

cepler.yml
```
environments:
  testflight:
    latest:
    - k8s/service.yml
    - k8s/testflight.yml
  staging:
    passed: testflight
    propagated:
    - k8s/service.yml
    latest:
    - k8s/staging.yml
  production:
    passed: staging
    propagated:
    - k8s/service.yml
    latest:
    - k8s/production.yml
```

There are 3 basic commands in cepler `check`, `prepare`, `record`.
- `cepler check -e <environment>` - Check if an environment needs deploying
- `cepler prepare -e <environment>` - Prepare the state of the files checked out in the current directory for deployment to <environment>
- `cepler record -e <environment>` -  Record (and commit) the state of files that are currently checked out

There are a number of additional cli flags described via `cepler help [subcommand]`:
```
$ cepler help
cepler 0.0.11

USAGE:
    cepler [OPTIONS] <SUBCOMMAND>

FLAGS:
    -h, --help       Prints help information
    -V, --version    Prints version information

OPTIONS:
        --clone <CLONE_DIR>                    Clone the repository into <dir>
    -c, --config <CONFIG_FILE>                 Cepler config file [env: CEPLER_CONF=]  [default: cepler.yml]
        --git-branch <GIT_BRANCH>              Branch for --clone option [env: GIT_BRANCH=]  [default: main]
        --git-private-key <GIT_PRIVATE_KEY>    Private key for --clone option [env: GIT_PRIVATE_KEY=]
        --git-url <GIT_URL>                    Remote url for --clone option [env: GIT_URL=]

SUBCOMMANDS:
    check        Check wether the environment needs deploying. Exit codes: 0 - needs deploying; 1 - internal error;
                 2 - nothing to deploy
    concourse    Subcommand for concourse integration
    help         Prints this message or the help of the given subcommand(s)
    prepare      Prepare workspace for hook execution
    record       Record the state of an environment in the statefile

$ cepler help check
cepler-check
Check wether the environment needs deploying. Exit codes: 0 - needs deploying; 1 - internal error; 2 - nothing to deploy

USAGE:
    cepler check --environment <ENVIRONMENT>

FLAGS:
    -h, --help    Prints help information

OPTIONS:
    -e, --environment <ENVIRONMENT>    The cepler environment [env: CEPLER_ENVIRONMENT=]
```
