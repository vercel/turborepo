# Three-shaking

## Summary

We’d like to get some preliminary form of DCE/tree shaking in Turbopack for the
purpose of SSR/SSG HMR. This RFC also details a more long term vision for tree
shaking in Turbopack.

## Motivation

We want Turbopack's tree shaking to be more granular than that of Webpack.
While Webpack will eliminate unused exports across the whole compilation
from a module, this module might end up duplicated across chunks, each chunk
only needing part of that module.

Instead, Tobias proposed Turbopack should split all module-level
declarations into their own modules. This way, each chunk can include only
the declarations it needs, and these declaration modules can be shared
across chunks.

This tree shaking implementation is primarily a concern for production
builds. We don't need a fully-fledged implementation for development builds.

However, we still need some form of tree shaking for eliminating
SSG and SSR specific functions from pages (`getServerSideProps`, etc.). This
is already implemented as the SSG transform (next_ssg.rs).

Similarly, for HMRing SSG and SSR, we need the complementary of this
operation: we want to eliminate all but SSG and SSR functions, so we can
ensure that we only re-render and update the server-side representation
when SSG and SSR functions change.

This requires a form of tree shaking more advanced than the one we currently
have in the SSG transform.

## Implementation

### Compiler passes

The tree-shaking transform works in multiple passes:

#### 1: The analyzer pass.

This pass would build a directed, possibly-cyclic graph of dependencies
between identifiers in a module. The graph is built starting from exports
and leading back up to module declarations and imports.

The analyzer pass could be restricted to only consider some exports by
passing in an [`ExportPredicate`], but this hurts caching for that pass.

e.g. for the given JS code:

```js
const dog = "dog";
const cat = "cat";

export const dog = dog;
export const chimera = cat + dog;
```

The graph would look like this:

```text
╔═══════╗   ┌───┐
║chimera║──▶│cat│
╚═══════╝   └───┘
    │
    ▼
  ┌───┐     ╔═══╗
  │dog│◀────║dog║
  └───┘     ╚═══╝

── Local
══ Export
```

#### 2: The grouping pass

This pass groups declarations into disjoint sets.

Starting from the exports, follow all outgoing edges transitively and mark
all visited declarations as accessible from this export.

Then, starting from the exports again, follow all outgoing edges. When
visiting a declaration, move it to the set identified by the set of exports
it is accessible from. For instance, if declaration A is accessible from
exports B and C, it will go into the set (B, C). However, if the declaration
itself is another export, then stop there.

```text
 ┌ ─ ─chimera─ ─ ┐
  ╔═══════╗ ┌───┐
 │║chimera║ │cat││
  ╚═══════╝ └───┘
 └ ─ ─ ─ ─ ─ ─ ─ ┘
             │
             ▼
 ┌ dog ┐  ┌(dog)┐
  ╔═══╗    ┌───┐
 │║dog║│─▶││dog││
  ╚═══╝    └───┘
 └ ─ ─ ┘  └ ─ ─ ┘

─x─ Declaration set
```

#### 3: Final pass

Given an [`ExportPredicate`], this pass will generate the final module
graph, where each declaration set identified in the previous pass will get
its own module. Dependencies between these modules are the same as the
edges in the graph.

Module (dog):

```js
export const virtual_dog = "dog";
```

Module dog:

```js
import { virtual_dog } from "(dog)";

export const dog = virtual_dog;
```

Module chimera:

```js
import { virtual_dog } from "(dog)";

const cat = "cat";

export const chimera = cat + virtual_dog;
```

#### A more complicated example

Consider the following module:

```js
let dog = "dog";

function getDog() {
    return dog;
}

function setDog(newDog) {
   setDog(newDog);
}

export const dogRef = {
    initial: dog
    get: getDog,
    set: setDog,
};

export let cat = "cat";

export const initialCat = cat;

export function getChimera() {
    return cat + dog;
}
```

This example showcases two kinds of dependencies between modules:

1. Live dependencies: `cat` is a live dependency of `getChimera`, since
   calling `getChimera` will always use the latest value of `cat`. The same
   applies to `dog` in `getChimera`, `getDog` and `setDog`.
2. Initial dependencies: `dog` is an initial dependency of `dogRef`, since
   the value of `dogRef.initial` is set to the initial value of `dog`. The
   same applies to `cat` in `initialCat`.

Now let's say our tree-shaking passes end up moving `cat` and `initialCat`
to different modules. If we kept a live dependency on `cat` in `initialCat`,
we could run into incorrect behavior if the value of `cat` is modified by
another module before `initialCat` is loaded and can read the initial value
of `cat`. As such, we need the `cat` module to export both an _initial_,
immutable value, and a _live_, mutable value.

Module cat:

```js
let cat = "cat";

// This could also use `Object.defineProperty(__turbopack_export_value__, ...)`.
export let live = cat;
export const initial = cat;
```

Module initialCat:

```js
import { initial as cat } from "cat";

export const initialCat = cat;
```

Module getChimera:

```js
import { live as cat } from "cat";
import { live as dog } from "dog";

export function getChimera() {
  return cat + dog;
}
```

#### Note

For the SSG and SSG-complementary operation, we don't actually need to run
the second pass, since we don't need to separate declarations into their
own modules. Instead, we will generate three modules:

1. The original module, with all declarations, for initial server-side SSR.
2. The client-side module, with SSG and SSR functions eliminated.
3. The server-side HMR module, with all but SSG and SSR functions
   eliminated.

For production, the second pass could also benefit from global information.
If we know which exports are used together, we can merge sets more
agressively. However, this might not be needed, as modules will be merged
together after chunking.

## Drawbacks

This will make the build process slower and more complex in a few ways:

1. The analyzer pass will slow down the _first_ processing of each module.
   However, in most cases, this pass will only happen in production builds.
2. ES modules can now be split into multiple, interdependent modules. This
   will significantly increases the total number of modules in the
   compilation.
3. Tree shaking doesn't play well with side effects, and errors in the
   implementation can have introduce subtle bugs. For instance, if a
   module is imported by a module that is not used, but the module has
   side effects, the side effects will not be executed.

## Prior art

1. [Rollup](https://rollupjs.org/guide/en/#tree-shaking)
2. [Webpack](https://webpack.js.org/guides/tree-shaking/)
3. [SWC DCE transform] which is used for bundling/minifying.

[swc dce transform]: https://github.com/swc-project/swc/blob/main/crates/swc_ecma_transforms_optimization/src/simplify/dce/mod.rs
[`exportpredicate`]: ./src/lib.rs
