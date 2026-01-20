Add a new shared package called @repo/config that exports a configuration object. Then update @repo/utils to depend on @repo/config.

1. Create packages/config/package.json with name "@repo/config"
2. Create packages/config/tsconfig.json
3. Create packages/config/src/index.ts that exports: { version: "1.0.0", env: "development" }
4. Add @repo/config as a dependency to @repo/utils package.json
