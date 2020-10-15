# [cepler release v0.4.4](https://github.com/bodymindarts/cepler/releases/tag/v0.4.4)

## Bug Fix
- Also apply `MatchOptions` for when checking out files

# [cepler release v0.4.3](https://github.com/bodymindarts/cepler/releases/tag/v0.4.3)

## Bug Fix
- Also apply `MatchOptions` for propagated files.

# [cepler release v0.4.2](https://github.com/bodymindarts/cepler/releases/tag/v0.4.2)

## Bug Fix
- Use explicit `MatchOptions` when testing glob pattern:
  ```
  glob::MatchOptions {
      case_sensitive: true,
      require_literal_separator: true,
      require_literal_leading_dot: true,
  }
  ```

# [cepler release v0.4.1](https://github.com/bodymindarts/cepler/releases/tag/v0.4.1)

## Deprecation

- Remove `concourse gen` subcommand. For integration with concourse pipelines see the ongoing work at https://github.com/bodymindarts/cepler-templates

# [cepler release v0.4.0](https://github.com/bodymindarts/cepler/releases/tag/v0.4.0)

## Breaking Changes
- Encode where the file came from in the state file via `{env}/path/to/file`

# cepler release v0.3.0

*Yanked release*

## Breaking changes
- Change the schema of the persisted state file to differentiate between propagated and latest files

## Bug Fix
- Fix wording for `cepler concourse ci_out` command

# [cepler release v0.2.2](https://github.com/bodymindarts/cepler/releases/tag/v0.2.2)

## Bug Fixes

- Fix `Error: Couldn't find environment` after adding new environment to `cepler.yml`

# [cepler release v0.2.1](https://github.com/bodymindarts/cepler/releases/tag/v0.2.1)

## Bug Fixes
- Load cepler.yml from disk when constructing head deploy state

# [cepler release v0.2.0](https://github.com/bodymindarts/cepler/releases/tag/v0.2.0)

## Features
- Check determins head commit based on last commit that actually influenced the environment state (instead of current head)

## Improvements
- Nicer commit message.  When committing the state via `cepler record` the commit message shouldn't have a `!`
- Display added files in metadata.

# [cepler release v0.1.1](https://github.com/bodymindarts/cepler/releases/tag/v0.1.1)

- Report `crate_version` on stderr in concourse operations
- Add `ls` command - `cepler ls -e <environment>` lists all files tracked by the current config.

# [cepler release v0.1.0](https://github.com/bodymindarts/cepler/releases/tag/v0.1.0)

Initial release of cepler - the Capricious Environment Propagate(l)er
Alpha software!
Please look at the [README](./README.md) and the `cepler help [sub]` command.
