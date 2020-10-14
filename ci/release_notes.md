## Bug Fix
- Use explicit `MatchOptions` when testing glob pattern:
  ```
  glob::MatchOptions {
      case_sensitive: true,
      require_literal_separator: true,
      require_literal_leading_dot: true,
  }
  ```
