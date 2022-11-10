# `turbo-ignore`

To get started, use the following command as your [Ignored Build Step](https://vercel.com/docs/concepts/projects/overview#ignored-build-step):

```sh
$ npx turbo-ignore
```

This uses `turbo` to automatically determine if the current app has new changes that need to be deployed.

## Usage

Use `npx turbo-ignore --help` to see list of options:

```sh
turbo-ignore

Automatically ignore builds that have no changes

Usage:
  $ npx turbo-ignore [<workspace>] [flags...]

If <workspace> is not provided, it will be inferred from the "name"
field of the "package.json" located at the current working directory.

Flags:
  --filter-fallback   When deploying on Vercel, If no previously deployed SHA is available to compare against,
                      fallback to comparing against the previous commit [default: true]
  --help, -h          Show this help message
  --version, -v       Show the version of this script
```

## How it Works

`turbo-ignore` determines if a build should continue by analyzing the package dependency graph of the given workspace.

The _given workspace_ is determined by reading the "name" field in the "package.json" file located at the current working directory, or by passing in a workspace name as the first argument to `turbo-ignore`.

Next, it uses `turbo run build --dry` to determine if the given workspace, _or any dependencies of the workspace_, have changed since the previous commit.

**NOTE:** `turbo` determines dependencies from reading the dependency graph of the given workspace. This means a workspace **must** be listed as a `dependency` (or `devDependency`) in the given workspaces `package.json` for `turbo` to recognize it.

When deploying on [Vercel](https://vercel.com), `turbo-ignore` can make a more accurate decision by comparing between the current commit, and the last successfully deployed commit for the current branch.

**NOTE:** By default on Vercel, if the branch has not been deployed, `turbo-ignore` will fall back to comparing against the previous commit. To always deploy the first commit to a new branch, this fallback behavior can be disabled with `--filter-fallback=false`.

## Releasing

```sh
pnpm release
```

---

For more information about Turborepo, visit [turbo.build](https://turbo.build) and follow us on Twitter ([@turborepo](https://twitter.com/turborepo))!
