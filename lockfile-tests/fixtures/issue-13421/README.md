# turbo prune loses track of Yarn packageExtension edges, breaking `yarn install --immutable`

## Summary

`turbo prune <workspace>` computes which parts of the monorepo `yarn.lock`
are still needed by walking the dependency graph. That walk appears to be
based purely on the `dependencies:` fields that are **textually present**
in each package's own lockfile resolution block.

Yarn **packageExtensions** (built-in ones shipped with Yarn itself, or
custom ones from a project's `.yarnrc.yml`) inject an extra dependency into
a package's manifest *before* resolution — but the injected edge is **not**
persisted as text under the origin package's own lockfile entry. It only
shows up as an entirely separate, independently-merged descriptor block for
the target package.

Because of that, `turbo prune`'s graph walk can never "see" a
packageExtension-injected edge, in either direction:

- **Variant A** — pruning to a workspace whose *only* reason to need the
  extension's target package is the (invisible) extension edge: the whole
  target package is dropped from the pruned lockfile, even though it is
  still genuinely required at install time.
- **Variant B** — pruning to a workspace that needs the target package for
  an unrelated, real, textually-visible reason, while another (now pruned)
  workspace was the one whose extension had injected an extra range for
  the same package: the merged lockfile descriptor header is kept
  byte-for-byte unchanged, including the now-orphaned range from the
  removed workspace.

In both variants, a plain `yarn install` afterwards would rewrite the
lockfile entry (to add the missing package, or to narrow the stale merged
header) — and that rewrite is exactly what `yarn install --immutable`
forbids, so CI/Docker builds that rely on `turbo prune` output fail.

This reproduces with **no custom Yarn configuration at all** — only Yarn's
own built-in `packageExtensions` list (see
[`@yarnpkg/extensions`](https://github.com/yarnpkg/berry/blob/master/packages/yarnpkg-extensions/sources/index.ts)),
so it is not specific to any one project's setup.

## Setup

- `pkg-a` depends on `notistack@^3.0.0`.
  Yarn ships a built-in extension for it:
  `["notistack@^3.0.0", { dependencies: { csstype: "^3.0.10" } }]`
  (notistack does not declare a real dependency on `csstype` itself — its
  own lockfile entry only lists `clsx` and `goober`).
- `pkg-b` depends on `csstype@^3.0.2` directly, for an unrelated, real reason.
- Because both ranges resolve to the same `csstype` version, the full
  monorepo `yarn.lock` merges them into a single descriptor block:

  ```yaml
  "csstype@npm:^3.0.10, csstype@npm:^3.0.2":
    version: 3.2.3
    resolution: "csstype@npm:3.2.3"
  ```

  ...while `notistack`'s own entry shows no trace of that `^3.0.10` edge:

  ```yaml
  "notistack@npm:^3.0.0":
    version: 3.0.2
    resolution: "notistack@npm:3.0.2"
    dependencies:
      clsx: "npm:^1.1.0"
      goober: "npm:^2.0.33"
    # csstype is applied live via the packageExtension, but never written here
  ```

## Variant A: pruning to the workspace that only needs the package via the extension

```bash
yarn install
npx turbo prune pkg-a
cd out
yarn install --immutable
```

`out/yarn.lock` no longer contains **any** `csstype` resolution block at
all (`pkg-b`, the only textually-visible consumer, was pruned away, and
`notistack`'s extension-injected need for `csstype` is invisible to the
walk). Result:

```
➤ YN0000: ┌ Post-resolution validation
➤ YN0002: │ pkg-a@workspace:packages/pkg-a doesn't provide react (pe8a6ee), requested by notistack.
➤ YN0002: │ pkg-a@workspace:packages/pkg-a doesn't provide react-dom (p456600), requested by notistack.
➤ YN0000: │ @@ -53,8 +53,14 @@
➤ YN0028: │ +"csstype@npm:^3.0.10":
➤ YN0028: │ +  version: 3.2.3
➤ YN0028: │ +  resolution: "csstype@npm:3.2.3"
➤ YN0028: │ +  languageName: node
➤ YN0028: │ +  linkType: hard
➤ YN0028: │ +
➤ YN0028: │ The lockfile would have been modified by this install, which is explicitly forbidden.
➤ YN0000: └ Completed
➤ YN0000: · Failed with errors in 0s 138ms
```

(The `react`/`react-dom` peer warnings are pre-existing and unrelated to
the bug — `notistack` peer-depends on React and neither the toy repo nor
the pruned output provides it.)

## Variant B: pruning to the workspace that needs the package for a real, unrelated reason

```bash
yarn install
npx turbo prune pkg-b
cd out
yarn install --immutable
```

`out/yarn.lock` still contains the full merged header from the original
lockfile, unnarrowed, even though `notistack` (and with it, the only
consumer of the `^3.0.10` half) is completely gone from `out/package.json`
and `out/yarn.lock`:

```yaml
"csstype@npm:^3.0.10, csstype@npm:^3.0.2":   # ^3.0.10 is now orphaned
  version: 3.2.3
  resolution: "csstype@npm:3.2.3"
```

Result:

```
➤ YN0000: ┌ Post-resolution validation
➤ YN0000: │ @@ -46,9 +46,9 @@
➤ YN0028: │ -"csstype@npm:^3.0.10, csstype@npm:^3.0.2":
➤ YN0028: │ +"csstype@npm:^3.0.2":
➤ YN0000: │    version: 3.2.3
➤ YN0000: │    resolution: "csstype@npm:3.2.3"
➤ YN0028: │ The lockfile would have been modified by this install, which is explicitly forbidden.
➤ YN0000: └ Completed
➤ YN0000: · Failed with errors in 0s 35ms
```

## Variant C: custom `.yarnrc.yml` packageExtensions, same workspace keeps both sides

Variants A and B above use one of Yarn's own **built-in** packageExtensions
(`notistack@^3.0.0 -> csstype@^3.0.10`). For that specific pair, pruning to a
single workspace that contains *both* the extension's origin package
(`notistack`) and a real, independent consumer of the target package
(`csstype`) works fine — the merged header survives intact (see `pkg-c`,
which depends on both directly; `turbo prune pkg-c` keeps
`"csstype@npm:^3.0.10, csstype@npm:^3.0.2"` unchanged and `yarn install
--immutable` succeeds in `out/`).

That is *not* true for a **custom** `packageExtensions` entry declared in the
project's own `.yarnrc.yml`. `pkg-d` reproduces this with a trivial custom
extension:

```yaml
# .yarnrc.yml
packageExtensions:
  ansi-regex@*:
    dependencies:
      left-pad: "*"
```

```json
// packages/pkg-d/package.json
{
  "dependencies": {
    "ansi-regex": "5.0.1",
    "left-pad": "^1.3.0"
  }
}
```

Both `ansi-regex` (the extension's origin) and `left-pad` (needed both via
the extension's `*` and directly via `^1.3.0`) live in the *same* workspace,
and that workspace is the *only* thing being pruned to — nothing is removed
from the graph at all:

```bash
yarn install
grep left-pad yarn.lock
# "left-pad@npm:*, left-pad@npm:^1.3.0":

npx turbo prune pkg-d
grep left-pad out/yarn.lock
# "left-pad@npm:^1.3.0":              <-- the "*" descriptor is gone, even
#                                          though ansi-regex is still right
#                                          there in the pruned workspace

cd out && yarn install --immutable
# ➤ -"left-pad@npm:^1.3.0":
# ➤ +"left-pad@npm:*, left-pad@npm:^1.3.0":
# ➤ The lockfile would have been modified by this install, which is explicitly forbidden.
```

This suggests `turbo prune` evaluates Yarn's **built-in** extension list
(bundled with Yarn itself) when recomputing the reachable descriptor set,
but does **not** read the project's own `.yarnrc.yml` `packageExtensions` —
even though `.yarnrc.yml` itself is otherwise copied byte-for-byte into the
pruned output.

**This is the variant that hit us in production.** We have a custom
extension `ssh2@* -> node-gyp@*` (our real dependency is on a fork of `ssh2`
via a `resolutions` git override, but the same applies to a plain npm
`ssh2`). `ssh2` is a real, direct dependency of the pruned workspace (via
`ssh2-promise`) and stays in the pruned graph; `node-gyp` also has other
real, independent consumers in the same workspace (a `node-gyp` devDependency,
plus `bufferutil`/`cpu-features`'s own `node-gyp: latest` build-time
dependency) — structurally identical to `pkg-d` above. Because the merged
header entry lost its `*` descriptor, `yarn install --immutable` failed
after `turbo prune`, and — worse than variants A/B — because there was no
`node-gyp@npm:*` entry left in the pruned lockfile *at all*, a plain
`yarn install` (without `--immutable`) silently re-resolved it against the
registry and picked up a newer version (`13.0.1`) than the one pinned in the
full monorepo lockfile (`12.3.0`), silently drifting the dependency tree of
Docker/CI builds away from what `yarn install` produces from the full
repository.

## Expected result

`turbo prune` should recompute the reachable descriptor set for every
retained package from the *actual* resolved dependency graph (the same one
`yarn why` / `yarn install` see, packageExtensions included), not just from
the literal `dependencies:` text of each lockfile entry. That would mean:

- Variant A: `csstype` stays in the pruned lockfile, because it is still
  really needed.
- Variant B: the merged header narrows to `csstype@npm:^3.0.2` only,
  because `^3.0.10` is no longer requested by anything reachable.
- Variant C: the merged header for `left-pad` stays
  `"left-pad@npm:*, left-pad@npm:^1.3.0"` unchanged, because both descriptors
  are still genuinely reachable inside the one retained workspace.

Either way, `yarn install --immutable` should then succeed on the pruned
output, matching what a plain `yarn install` would produce from scratch.

## Why this matters beyond this toy example

This is not specific to `notistack`/`csstype`/`ansi-regex`/`left-pad`. It
reproduces with:

- Any Yarn built-in `packageExtension`
  (see the full list in
  [`@yarnpkg/extensions`](https://github.com/yarnpkg/berry/blob/master/packages/yarnpkg-extensions/sources/index.ts))
  — variants A and B.
- Custom `packageExtensions` configured in a project's own `.yarnrc.yml` —
  variant C, which is strictly worse: it doesn't even require pruning away
  any package. A single retained workspace that both depends on the
  extension's origin package and separately, genuinely needs the target
  package is enough to lose the merged descriptor.
- This is exactly the shape of our real, non-toy case: a custom
  `ssh2@* -> node-gyp@*` extension, where `ssh2` (via `ssh2-promise`) and a
  real `node-gyp` devDependency both live in the same pruned server
  workspace. Losing the `node-gyp@npm:*` descriptor there was worse than a
  mere `--immutable` failure — since no `node-gyp@npm:*` entry remained in
  the pruned lockfile at all, a plain `yarn install` silently re-resolved it
  to a newer version than the one pinned by the full monorepo install.

## Environment

- `yarn`: 4.17.0
- `turbo`: 2.10.5 — variant C additionally confirmed reproducing on
  `2.10.6-canary.4` (latest canary at the time of writing)
- `node`: v24.18.0
- OS: Windows 11
