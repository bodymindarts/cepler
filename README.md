# CEPler

Capricious Environment Propagator is here to help you manage state of files representing a system deployed to multiple environments.

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
  # The Name of the Environment
  testflight:

    # List of files (globs) that should (always) trigger a redeploy of this environment
    latest:
    - k8s/service.yml
    - k8s/testflight.yml

  staging:
    # The preceeding environment
    passed: testflight
    # The files that should trigger once they have been propagated from the preceeding environment
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
- `cepler prepare -e <environment>` - Prepare the state of the files checked out in the current directory for deployment
- `cepler record -e <environment>` -  Record (and commit) metadata about files currently checked out and relevant to the environment

There are a number of additional cli flags described via `cepler help [subcommand]`:
```
$ cepler --help
cepler 0.4.8

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
    ls           List all files relevent to a given environment
    prepare      Prepare workspace for hook execution
    record       Record the state of an environment in the statefile
    
$ cepler help prepare
cepler-prepare
Prepare workspace for hook execution

USAGE:
    cepler prepare [FLAGS] --environment <ENVIRONMENT>

FLAGS:
        --force-clean    Delete all files not referenced in cepler.yml
    -h, --help           Prints help information

OPTIONS:
    -e, --environment <ENVIRONMENT>    The cepler environment [env: CEPLER_ENVIRONMENT=]
```

## Concourse

For information on integration into concourse pipelines refer to the readme at [concourse/README.md](concourse/README.md)
