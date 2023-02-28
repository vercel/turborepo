# Release Documentation

## Release npm packages

We have a simple script to release npm packages from pnpm workspaces: `cargo xtask workspace --bump`.

```text
? Select a package to bump
> [ ] @vercel/node-module-trace
  [ ] @vercel/webpack-node-module-trace
[↑↓ to move, space to select one, → to all, ← to none, type to filter]
```

Press space to select the package you want to publish.
Press enter to choose the version type you want to bump:

```text
? Select a package to bump @vercel/node-module-trace, @vercel/webpack-node-module-trace
? Version for @vercel/node-module-trace
  patch
  minor
> major
  alpha
  beta
  canary
[↑↓ to move, enter to select, type to filter]
```

> **Note**
>
> This command will always increase the version according to the semver version. <br/>
> For example, if the current version of one package is `1.0.0`, and you choose `patch`, the version will be increased to `1.0.1`. <br/>

> **Warning**
>
> If the version of one package is `1.0.0-beta.0`, and you choose `alpha`, the cli will panic and exit. Because the `beta` < `alpha` in semver.

Once you have finished the bump, the script will do the following things:

- bump the version you choose in the corresponding package
- update dependencies in other packages that depend on the package you choose
- update `pnpm-lock.yaml` file
- run `git tag -s pkg@version -m "pkg@version"` for each package

You need to run `git push --follow-tags` to finish the release.

## Release Turborepo

We have a multi step release process for Turborepo right now.

**NOTE**: The steps below _must_ be run serially, in the order specified.

1. Create a release branch by triggering the [1. Turborepo Release (release branch)][1] workflow

   - Specify the semver increment using the SemVer Increment field (start with `prerelease`)

2. Build the Go Binary by triggering the [2. Turborepo Release (go binary)][2] workflow.

   1. Specify the release branch (example: `staging-1.7.0-canary.1`) in _both_ the "use workflow from", and "Staging branch to release from" fields.

3. Build the Rust Wrapper by triggering the [3. Turborepo Release (rust binary & publish)][3] workflow.
   1. Specify the release branch (example: `staging-1.7.0-canary.1`) in _both_ the "use workflow from", and "Staging branch to release from" fields. (this should match step 2.1 above)
4. A PR is automatically opened to merge the release branch created in step 1 back into `main`

   1. ⚠️ Merge this in! You don't need to wait for tests to pass.

   It's important to merge this branch soon after the publish is succesful

5. `turbo-orchestrator.yml` polls `npm` every 5 mins. When a new version is detected,
   [`turborepo-smoke-published.yml`][4] runs against `@latest` and `@canary` tags.

### Notes

- Github Release Notes are published on their own using config from `turborepo-release.yml`,
  triggered by the `turbo-orchestrator` bot.
- `eslint-plugin-turbo` and `eslint-config-turbo` need to be published separately.

[1]: https://github.com/vercel/turbo/actions/workflows/turborepo-release-step-1.yml
[2]: https://github.com/vercel/turbo/actions/workflows/turborepo-release-step-2.yml
[3]: https://github.com/vercel/turbo/actions/workflows/turborepo-release-step-3.yml
[3]: https://github.com/vercel/turbo/actions/workflows/turborepo-smoke-published.yml
