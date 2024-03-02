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

1. Create a release by triggering the [1. Turborepo Release][1] workflow

   - Specify the semver increment using the SemVer Increment field (start with `prerelease`)
   - Check the "Dry Run" box to run the full release workflow without publishing any packages.

2. A PR is automatically opened to merge the release branch created in step 1 back into `main`

   - ⚠️ Merge this in! You don't need to wait for tests to pass. It's important to merge this branch soon after the publish is successful

### Notes

- Github Release Notes are published automatically using the config from [`turborepo-release.yml`][2],
  triggered by the [`turbo-orchestrator`][3] bot.

[1]: https://github.com/vercel/turbo/actions/workflows/turborepo-release.yml
[2]: https://github.com/vercel/turbo/blob/main/.github/turborepo-release.yml
[3]: https://github.com/apps/turbo-orchestrator
