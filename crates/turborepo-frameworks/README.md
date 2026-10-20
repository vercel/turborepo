# turborepo-frameworks

## Purpose

Framework detection and configuration inference. Identifies JavaScript frameworks (Next.js, Vite, etc.) and determines which environment variables affect them.

## Architecture

```
package.json dependencies
    └── turborepo-frameworks
        ├── Match against framework signatures
        └── Return framework-specific env wildcards
            e.g., NEXT_PUBLIC_*, VITE_*
```

Detected frameworks include:
- Next.js
- Vite
- Create React App
- Gatsby
- And others...

## Notes

Framework detection enables automatic inclusion of framework-specific environment variables in task hashes. This prevents cache misses when framework env vars change.
