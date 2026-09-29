## Repositories

Provides Turborepo repository analysis for JavaScript, Cargo, uv, and Go workspaces
from a JavaScript API. Native discovery follows the corresponding `turbo.json`
future flags: `experimentalCargoWorkspaces`, `experimentalPythonWorkspaces`, and
`experimentalGoWorkspaces`.

Note that this is not yet a stable package, and functionality, API, and naming may all change.

## Static discovery

Use `StaticWorkspace` for a package inventory and conservative affected candidates
without invoking Cargo, rustc, uv, Python, or Go. It still requires the
platform-specific `.node` addon; it is not a pure-JavaScript implementation.

```js
import { StaticWorkspace } from "@turbo/repository";

const workspace = await StaticWorkspace.find(); // Or pass a directory.
const packages = await workspace.findPackages();
const roots = workspace.workspaceRoots();
const { packages: candidates, conservative } =
  await workspace.affectedCandidates(["packages/ui/src/button.ts"]);

console.log(workspace.absolutePath, workspace.dependencyGraphComplete);
console.log(
  workspace.unloadedToolchains,
  packages,
  roots,
  candidates,
  conservative
);
```

`find(path?)` uses the same JavaScript workspace-root and package-manager inference
as `Workspace.find`, including lockfile-based inference when the package manager
is undeclared. It does not add standalone native-only root inference.

Packages are plain objects with `name`, `absolutePath`, `relativePath`,
`manifestPath` (workspace-relative), and `toolchain`. Co-located packages are
preserved; the repository root and native workspace aggregates are excluded.
Packages are sorted by `relativePath`, then `toolchain`, then `name`.
`workspaceRoots()` returns plain `{ toolchain, kind, relativePath }` objects.
Language IDs are `javascript`, `rust`, `python`, and `go`; native root kinds are
`cargo`, `uv`, and `go`.

`dependencyGraphComplete` remains false while native authoritative metadata is
unloaded. The independent `affectednessComplete` getter indicates whether declared
local package inputs can be inferred statically. Do not use the former to disable
selective native affectedness.

Cargo path dependencies include workspace inheritance, aliases, build/dev inputs,
optional features, and every target-conditional branch. uv inputs include
workspace sources, local paths to workspace members, normalized names, extras,
dependency groups, and the union of conditional source choices. Member source
overrides take precedence over root sources. Co-located package scopes propagate
changes between ecosystems. No compiler, resolver, or language command runs.

Source changes select their package owners and transitive dependents. For example,
a Rust library change selects its Rust consumers but not unrelated Python or
JavaScript packages. Native workspace lockfiles and toolchain configuration
invalidate the corresponding native workspace and its dependents. Manifest edits
in any ecosystem and nested JavaScript lockfile edits still select all packages:
removed/renamed scopes can erase historical cross-language relationships, including
connections through co-located package scopes.

Unresolved cases return all packages with `conservative: true`: unsupported
contributors (currently Go), Cargo patch/replace or source/path overrides,
non-member local dependencies, Python dynamic dependencies or dependency metadata
overrides, and explicit Maturin manifest-path configuration. Inspect
`affectednessComplete` rather than assuming every unloaded toolchain is unresolved.
Global inputs only trigger fallback when a changed path matches; exclusions are
ignored conservatively and unsupported patterns fall back to all.

These are repository-declared package inputs, not arbitrary build-script reads,
external dependency resolution, task ordering, or exact target/feature selection.
An empty change list always returns `{ packages: [], conservative: false }`.
Changed paths must be workspace-relative, using the current system's path
separator; absolute paths and paths escaping the repository are rejected. The
inventory describes the current checkout, not deleted packages from an older
revision. Deployment consumers must treat configured project roots missing from
the inventory as unknown/affected, rather than interpreting absence as a safe skip.
Malformed manifests reject discovery; deployment consumers should fail open on
those errors as well.

`StaticWorkspace` has no full graph, task, or lockfile methods. Use `Workspace`
when authoritative dependency precision is needed; full native discovery remains
toolchain-dependent. Existing `Workspace.find()` and its lockfile-only
`skipPackageGraph` option are unchanged.

## Development Notes

1.  `js/index.d.ts` is checked in and regenerated from the Rust N-API declarations by the development build. Update Rust API documentation alongside interface changes and include the regenerated declarations.
2.  If new exports are added, `index.js` will need to be updated as well.
