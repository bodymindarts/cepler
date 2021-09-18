## Feature

- `-g / --gates` flag for providing a commit per environment up to which preparations are allowed
- `--gates-branch` optional flag to checkout the gates file from another branch

The gates file is a yaml file with the names of the environments as keys and the complete git hahs as values:
```
staging: HEAD
production: d5739f9cb7ce6b1ff42cda0999c351790288cdc5
```
