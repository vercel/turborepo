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
  --fallback=<ref>    On Vercel, if no previously deployed SHA is available to compare against,
                      fallback to comparing against the provided ref [default: None]
  --help, -h          Show this help message
  --version, -v       Show the version of this script

---

turbo-ignore will also check for special commit messages to indicate if a build should be skipped or not.

Skip turbo-ignore check and automatically ignore:
  - [skip ci]
  - [ci skip]
  - [no ci]
  - [skip vercel]
  - [vercel skip]
  - [vercel skip <workspace>]

Skip turbo-ignore check and automatically deploy:
  - [vercel deploy]
  - [vercel build]
  - [vercel deploy <workspace>]
  - [vercel build <workspace>]
```

### Examples

```sh
npx turbo-ignore
```

> Only build if there are changes to the workspace in the current working directory, or any of it's dependencies. On Vercel, compare against the last successful deployment for the current branch. When not on Vercel, compare against the parent commit (`HEAD^`).

---

```sh
npx turbo-ignore docs
```

> Only build if there are changes to the `docs` workspace, or any of its dependencies. On Vercel, compare against the last successful deployment for the current branch. When not on Vercel compare against the parent commit (`HEAD^`).

---

```sh
npx turbo-ignore --fallback=HEAD~10
```

> Only build if there are changes to the workspace in the current working directory, or any of it's dependencies. On Vercel, compare against the last successful deployment for the current branch. If this does not exist (first deploy of the branch), compare against the previous 10 commits. When not on Vercel, always compare against the parent commit (`HEAD^`).

---

```sh
npx turbo-ignore --fallback=HEAD^
```

> Only build if there are changes to the workspace in the current working directory, or any of it's dependencies. On Vercel, compare against the last successful deployment for the current branch. If this does not exist (first deploy of the branch), compare against the parent commit (`HEAD^`). When not on Vercel, always compare against the parent commit (`HEAD^`).

## How it Works

`turbo-ignore` determines if a build should continue by analyzing the package dependency graph of the given workspace.

The _given workspace_ is determined by reading the "name" field in the "package.json" file located at the current working directory, or by passing in a workspace name as the first argument to `turbo-ignore`.

Next, it uses `turbo run build --dry` to determine if the given workspace, _or any dependencies of the workspace_, have changed since the previous commit.

**NOTE:** `turbo` determines dependencies from reading the dependency graph of the given workspace. This means a workspace **must** be listed as a `dependency` (or `devDependency`) in the given workspaces `package.json` for `turbo` to recognize it.

When deploying on [Vercel](https://vercel.com), `turbo-ignore` can make a more accurate decision by comparing between the current commit, and the last successfully deployed commit for the current branch.

**NOTE:** By default on Vercel, `turbo-ignore` will always deploy the first commit of a new branch. This behavior can be changed by providing the `ref` to compare against to the `--fallback` flag. See the [Examples](#Examples) section for more details.

---

For more information about Turborepo, visit [turbo.build](https://turbo.build) and follow us on Twitter ([@turborepo](https://twitter.com/turborepo))!
