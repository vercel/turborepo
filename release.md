# Release Documentation

1. Create a release by triggering the [Turborepo Release][1] workflow

   - Specify the semver increment using the SemVer Increment field (start with `prerelease`)
   - Check the "Dry Run" box to run the full release workflow without publishing any packages.

2. A PR is automatically opened to merge the release branch created in step 1 back into `main`

   - ⚠️ Merge this in! You don't need to wait for tests to pass. It's important to merge this branch soon after the
     publish is successful

### Notes

- GitHub Release Notes are published automatically using the config from [`turborepo-release.yml`][2],
  triggered by the [`turbo-orchestrator`][3] bot.

## Release `@turbo/repository`

1. Run [`bump-version.sh`][4] to update the versions of the packages. Merge in the changes to `main`.

2. Create a release by triggering the [Turborepo Library Release][5] workflow.
   - Check the "Dry Run" box to run the full release workflow without publishing any packages.

[1]: https://github.com/vercel/turborepo/actions/workflows/turborepo-release.yml
[2]: https://github.com/vercel/turborepo/blob/main/.github/turborepo-release.yml
[3]: https://github.com/apps/turbo-orchestrator
[4]: https://github.com/vercel/turborepo/blob/main/packages/turbo-repository/scripts/bump-version.sh
[5]: https://github.com/vercel/turborepo/actions/workflows/turborepo-library-release.yml
