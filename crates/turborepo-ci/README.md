# turborepo-ci

## Purpose

CI/CD environment detection and vendor-specific behavior. Detects which CI system is running and provides vendor-specific functionality.

## Architecture

```
Environment variables
    └── turborepo-ci
        ├── is_ci() - Detect if running in CI
        └── Vendor::infer() - Identify specific CI vendor
            ├── GitHub Actions
            ├── GitLab CI
            ├── CircleCI
            ├── Jenkins
            └── Many others...
```

Per-vendor information:
- Commit SHA environment variable
- Branch name environment variable
- Username environment variable
- Log grouping behavior

## Notes

Detection uses environment variables that each CI system sets. The vendor-specific behavior enables proper log grouping and other CI integrations.
