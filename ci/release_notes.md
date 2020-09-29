## Breaking Changes
- Encode where the file came from in the state file via `{env}/path/to/file`

## Bug Fixes
- only persist latest/propagated files when present (don't fail when key is missing)
