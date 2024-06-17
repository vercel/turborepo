# Items

Count: 28

## Item 10: Stmt 0, `ImportOfModule`

```js
import React from 'react';

```

- Hoisted
- Side effects

## Item 11: Stmt 0, `ImportBinding(0)`

```js
import React from 'react';

```

- Hoisted
- Declares: `React`

## Item 12: Stmt 1, `ImportOfModule`

```js
import { DynamicServerError } from '../../client/components/hooks-server-context';

```

- Hoisted
- Side effects

## Item 13: Stmt 1, `ImportBinding(0)`

```js
import { DynamicServerError } from '../../client/components/hooks-server-context';

```

- Hoisted
- Declares: `DynamicServerError`

## Item 14: Stmt 2, `ImportOfModule`

```js
import { StaticGenBailoutError } from '../../client/components/static-generation-bailout';

```

- Hoisted
- Side effects

## Item 15: Stmt 2, `ImportBinding(0)`

```js
import { StaticGenBailoutError } from '../../client/components/static-generation-bailout';

```

- Hoisted
- Declares: `StaticGenBailoutError`

## Item 16: Stmt 3, `ImportOfModule`

```js
import { getPathname } from '../../lib/url';

```

- Hoisted
- Side effects

## Item 17: Stmt 3, `ImportBinding(0)`

```js
import { getPathname } from '../../lib/url';

```

- Hoisted
- Declares: `getPathname`

## Item 18: Stmt 4, `VarDeclarator(0)`

```js
const hasPostpone = typeof React.unstable_postpone === 'function';

```

- Declares: `hasPostpone`
- Reads: `React`
- Write: `hasPostpone`, `React`

## Item 19: Stmt 5, `Normal`

```js
export function createPrerenderState(isDebugSkeleton) {
    return {
        isDebugSkeleton,
        dynamicAccesses: []
    };
}

```

- Hoisted
- Declares: `createPrerenderState`
- Write: `createPrerenderState`

## Item 20: Stmt 6, `Normal`

```js
export function markCurrentScopeAsDynamic(store, expression) {
    const pathname = getPathname(store.urlPathname);
    if (store.isUnstableCacheCallback) {
        return;
    } else if (store.dynamicShouldError) {
        throw new StaticGenBailoutError(`Route ${pathname} with \`dynamic = "error"\` couldn't be rendered statically because it used \`${expression}\`. See more info here: https://nextjs.org/docs/app/building-your-application/rendering/static-and-dynamic#dynamic-rendering`);
    } else if (store.prerenderState) {
        postponeWithTracking(store.prerenderState, expression, pathname);
    } else {
        store.revalidate = 0;
        if (store.isStaticGeneration) {
            const err = new DynamicServerError(`Route ${pathname} couldn't be rendered statically because it used ${expression}. See more info here: https://nextjs.org/docs/messages/dynamic-server-error`);
            store.dynamicUsageDescription = expression;
            store.dynamicUsageStack = err.stack;
            throw err;
        }
    }
}

```

- Hoisted
- Declares: `markCurrentScopeAsDynamic`
- Reads (eventual): `getPathname`, `StaticGenBailoutError`, `postponeWithTracking`, `DynamicServerError`
- Write: `markCurrentScopeAsDynamic`

## Item 21: Stmt 7, `Normal`

```js
export function trackDynamicDataAccessed(store, expression) {
    const pathname = getPathname(store.urlPathname);
    if (store.isUnstableCacheCallback) {
        throw new Error(`Route ${pathname} used "${expression}" inside a function cached with "unstable_cache(...)". Accessing Dynamic data sources inside a cache scope is not supported. If you need this data inside a cached function use "${expression}" outside of the cached function and pass the required dynamic data in as an argument. See more info here: https://nextjs.org/docs/app/api-reference/functions/unstable_cache`);
    } else if (store.dynamicShouldError) {
        throw new StaticGenBailoutError(`Route ${pathname} with \`dynamic = "error"\` couldn't be rendered statically because it used \`${expression}\`. See more info here: https://nextjs.org/docs/app/building-your-application/rendering/static-and-dynamic#dynamic-rendering`);
    } else if (store.prerenderState) {
        postponeWithTracking(store.prerenderState, expression, pathname);
    } else {
        store.revalidate = 0;
        if (store.isStaticGeneration) {
            const err = new DynamicServerError(`Route ${pathname} couldn't be rendered statically because it used ${expression}. See more info here: https://nextjs.org/docs/messages/dynamic-server-error`);
            store.dynamicUsageDescription = expression;
            store.dynamicUsageStack = err.stack;
            throw err;
        }
    }
}

```

- Hoisted
- Declares: `trackDynamicDataAccessed`
- Reads (eventual): `getPathname`, `StaticGenBailoutError`, `postponeWithTracking`, `DynamicServerError`
- Write: `trackDynamicDataAccessed`

## Item 22: Stmt 8, `Normal`

```js
export function Postpone({ reason, prerenderState, pathname }) {
    postponeWithTracking(prerenderState, reason, pathname);
}

```

- Hoisted
- Declares: `Postpone`
- Reads (eventual): `postponeWithTracking`
- Write: `Postpone`

## Item 23: Stmt 9, `Normal`

```js
export function trackDynamicFetch(store, expression) {
    if (!store.prerenderState || store.isUnstableCacheCallback) return;
    postponeWithTracking(store.prerenderState, expression, store.urlPathname);
}

```

- Hoisted
- Declares: `trackDynamicFetch`
- Reads (eventual): `postponeWithTracking`
- Write: `trackDynamicFetch`

## Item 24: Stmt 10, `Normal`

```js
function postponeWithTracking(prerenderState, expression, pathname) {
    assertPostpone();
    const reason = `Route ${pathname} needs to bail out of prerendering at this point because it used ${expression}. ` + `React throws this special object to indicate where. It should not be caught by ` + `your own try/catch. Learn more: https://nextjs.org/docs/messages/ppr-caught-error`;
    prerenderState.dynamicAccesses.push({
        stack: prerenderState.isDebugSkeleton ? new Error().stack : undefined,
        expression
    });
    React.unstable_postpone(reason);
}

```

- Hoisted
- Declares: `postponeWithTracking`
- Reads (eventual): `assertPostpone`, `React`
- Write: `postponeWithTracking`
- Write (eventual): `React`

## Item 25: Stmt 11, `Normal`

```js
export function usedDynamicAPIs(prerenderState) {
    return prerenderState.dynamicAccesses.length > 0;
}

```

- Hoisted
- Declares: `usedDynamicAPIs`
- Write: `usedDynamicAPIs`

## Item 26: Stmt 12, `Normal`

```js
export function formatDynamicAPIAccesses(prerenderState) {
    return prerenderState.dynamicAccesses.filter((access)=>typeof access.stack === 'string' && access.stack.length > 0).map(({ expression, stack })=>{
        stack = stack.split('\n').slice(4).filter((line)=>{
            if (line.includes('node_modules/next/')) {
                return false;
            }
            if (line.includes(' (<anonymous>)')) {
                return false;
            }
            if (line.includes(' (node:')) {
                return false;
            }
            return true;
        }).join('\n');
        return `Dynamic API Usage Debug - ${expression}:\n${stack}`;
    });
}

```

- Hoisted
- Declares: `formatDynamicAPIAccesses`
- Write: `formatDynamicAPIAccesses`

## Item 27: Stmt 13, `Normal`

```js
function assertPostpone() {
    if (!hasPostpone) {
        throw new Error(`Invariant: React.unstable_postpone is not defined. This suggests the wrong version of React was loaded. This is a bug in Next.js`);
    }
}

```

- Hoisted
- Declares: `assertPostpone`
- Reads (eventual): `hasPostpone`
- Write: `assertPostpone`

## Item 28: Stmt 14, `Normal`

```js
export function createPostponedAbortSignal(reason) {
    assertPostpone();
    const controller = new AbortController();
    try {
        React.unstable_postpone(reason);
    } catch (x) {
        controller.abort(x);
    }
    return controller.signal;
}

```

- Hoisted
- Declares: `createPostponedAbortSignal`
- Reads (eventual): `assertPostpone`, `React`
- Write: `createPostponedAbortSignal`
- Write (eventual): `React`

# Phase 1
```mermaid
graph TD
    Item5;
    Item5["ModuleEvaluation"];
    Item6;
    Item6["export Postpone"];
    Item7;
    Item7["export createPostponedAbortSignal"];
    Item8;
    Item8["export createPrerenderState"];
    Item9;
    Item9["export formatDynamicAPIAccesses"];
    Item10;
    Item10["export markCurrentScopeAsDynamic"];
    Item11;
    Item11["export trackDynamicDataAccessed"];
    Item12;
    Item12["export trackDynamicFetch"];
    Item13;
    Item13["export usedDynamicAPIs"];
    Item1;
    Item14;
    Item2;
    Item15;
    Item3;
    Item16;
    Item4;
    Item17;
    Item18;
    Item19;
    Item20;
    Item21;
    Item22;
    Item23;
    Item24;
    Item25;
    Item26;
    Item27;
    Item28;
    Item2 --> Item1;
    Item3 --> Item1;
    Item3 --> Item2;
    Item4 --> Item1;
    Item4 --> Item2;
    Item4 --> Item3;
```
# Phase 2
```mermaid
graph TD
    Item5;
    Item5["ModuleEvaluation"];
    Item6;
    Item6["export Postpone"];
    Item7;
    Item7["export createPostponedAbortSignal"];
    Item8;
    Item8["export createPrerenderState"];
    Item9;
    Item9["export formatDynamicAPIAccesses"];
    Item10;
    Item10["export markCurrentScopeAsDynamic"];
    Item11;
    Item11["export trackDynamicDataAccessed"];
    Item12;
    Item12["export trackDynamicFetch"];
    Item13;
    Item13["export usedDynamicAPIs"];
    Item1;
    Item14;
    Item2;
    Item15;
    Item3;
    Item16;
    Item4;
    Item17;
    Item18;
    Item19;
    Item20;
    Item21;
    Item22;
    Item23;
    Item24;
    Item25;
    Item26;
    Item27;
    Item28;
    Item2 --> Item1;
    Item3 --> Item1;
    Item3 --> Item2;
    Item4 --> Item1;
    Item4 --> Item2;
    Item4 --> Item3;
    Item6 --> Item22;
    Item7 --> Item28;
    Item8 --> Item19;
    Item9 --> Item26;
    Item10 --> Item20;
    Item11 --> Item21;
    Item12 --> Item23;
    Item13 --> Item25;
    Item18 --> Item14;
```
# Phase 3
```mermaid
graph TD
    Item5;
    Item5["ModuleEvaluation"];
    Item6;
    Item6["export Postpone"];
    Item7;
    Item7["export createPostponedAbortSignal"];
    Item8;
    Item8["export createPrerenderState"];
    Item9;
    Item9["export formatDynamicAPIAccesses"];
    Item10;
    Item10["export markCurrentScopeAsDynamic"];
    Item11;
    Item11["export trackDynamicDataAccessed"];
    Item12;
    Item12["export trackDynamicFetch"];
    Item13;
    Item13["export usedDynamicAPIs"];
    Item1;
    Item14;
    Item2;
    Item15;
    Item3;
    Item16;
    Item4;
    Item17;
    Item18;
    Item19;
    Item20;
    Item21;
    Item22;
    Item23;
    Item24;
    Item25;
    Item26;
    Item27;
    Item28;
    Item2 --> Item1;
    Item3 --> Item1;
    Item3 --> Item2;
    Item4 --> Item1;
    Item4 --> Item2;
    Item4 --> Item3;
    Item6 --> Item22;
    Item7 --> Item28;
    Item8 --> Item19;
    Item9 --> Item26;
    Item10 --> Item20;
    Item11 --> Item21;
    Item12 --> Item23;
    Item13 --> Item25;
    Item18 --> Item14;
    Item20 --> Item17;
    Item20 --> Item16;
    Item20 --> Item24;
    Item20 --> Item15;
    Item21 --> Item17;
    Item21 --> Item16;
    Item21 --> Item24;
    Item21 --> Item15;
    Item22 --> Item24;
    Item23 --> Item24;
    Item24 --> Item27;
    Item24 --> Item18;
    Item27 --> Item18;
    Item28 --> Item27;
    Item28 --> Item18;
```
# Phase 4
```mermaid
graph TD
    Item5;
    Item5["ModuleEvaluation"];
    Item6;
    Item6["export Postpone"];
    Item7;
    Item7["export createPostponedAbortSignal"];
    Item8;
    Item8["export createPrerenderState"];
    Item9;
    Item9["export formatDynamicAPIAccesses"];
    Item10;
    Item10["export markCurrentScopeAsDynamic"];
    Item11;
    Item11["export trackDynamicDataAccessed"];
    Item12;
    Item12["export trackDynamicFetch"];
    Item13;
    Item13["export usedDynamicAPIs"];
    Item1;
    Item14;
    Item2;
    Item15;
    Item3;
    Item16;
    Item4;
    Item17;
    Item18;
    Item19;
    Item20;
    Item21;
    Item22;
    Item23;
    Item24;
    Item25;
    Item26;
    Item27;
    Item28;
    Item2 --> Item1;
    Item3 --> Item1;
    Item3 --> Item2;
    Item4 --> Item1;
    Item4 --> Item2;
    Item4 --> Item3;
    Item6 --> Item22;
    Item7 --> Item28;
    Item8 --> Item19;
    Item9 --> Item26;
    Item10 --> Item20;
    Item11 --> Item21;
    Item12 --> Item23;
    Item13 --> Item25;
    Item18 --> Item14;
    Item20 --> Item17;
    Item20 --> Item16;
    Item20 --> Item24;
    Item20 --> Item15;
    Item21 --> Item17;
    Item21 --> Item16;
    Item21 --> Item24;
    Item21 --> Item15;
    Item22 --> Item24;
    Item23 --> Item24;
    Item24 --> Item27;
    Item24 --> Item18;
    Item27 --> Item18;
    Item28 --> Item27;
    Item28 --> Item18;
    Item5 --> Item1;
    Item5 --> Item2;
    Item5 --> Item3;
    Item5 --> Item4;
```
# Final
```mermaid
graph TD
    N0["Items: [ItemId(ModuleEvaluation), ItemId(0, ImportOfModule), ItemId(1, ImportOfModule), ItemId(2, ImportOfModule), ItemId(3, ImportOfModule)]"];
    N1["Items: [ItemId(Export((&quot;Postpone&quot;, #2), &quot;Postpone&quot;)), ItemId(8, Normal)]"];
    N2["Items: [ItemId(Export((&quot;createPostponedAbortSignal&quot;, #2), &quot;createPostponedAbortSignal&quot;)), ItemId(14, Normal)]"];
    N3["Items: [ItemId(Export((&quot;createPrerenderState&quot;, #2), &quot;createPrerenderState&quot;)), ItemId(5, Normal)]"];
    N4["Items: [ItemId(Export((&quot;formatDynamicAPIAccesses&quot;, #2), &quot;formatDynamicAPIAccesses&quot;)), ItemId(12, Normal)]"];
    N5["Items: [ItemId(Export((&quot;markCurrentScopeAsDynamic&quot;, #2), &quot;markCurrentScopeAsDynamic&quot;)), ItemId(1, ImportBinding(0)), ItemId(2, ImportBinding(0)), ItemId(3, ImportBinding(0)), ItemId(6, Normal)]"];
    N6["Items: [ItemId(Export((&quot;trackDynamicDataAccessed&quot;, #2), &quot;trackDynamicDataAccessed&quot;)), ItemId(1, ImportBinding(0)), ItemId(2, ImportBinding(0)), ItemId(3, ImportBinding(0)), ItemId(7, Normal)]"];
    N7["Items: [ItemId(Export((&quot;trackDynamicFetch&quot;, #2), &quot;trackDynamicFetch&quot;)), ItemId(9, Normal)]"];
    N8["Items: [ItemId(Export((&quot;usedDynamicAPIs&quot;, #2), &quot;usedDynamicAPIs&quot;)), ItemId(11, Normal)]"];
    N9["Items: [ItemId(0, ImportBinding(0)), ItemId(4, VarDeclarator(0))]"];
    N10["Items: [ItemId(10, Normal)]"];
    N11["Items: [ItemId(13, Normal)]"];
    N1 --> N10;
    N2 --> N11;
    N2 --> N9;
    N5 --> N6;
    N5 --> N10;
    N6 --> N10;
    N7 --> N10;
    N10 --> N11;
    N10 --> N9;
    N11 --> N9;
```
# Entrypoints

```
{
    ModuleEvaluation: 0,
    Export(
        "createPrerenderState",
    ): 3,
    Export(
        "markCurrentScopeAsDynamic",
    ): 5,
    Export(
        "usedDynamicAPIs",
    ): 8,
    Export(
        "Postpone",
    ): 1,
    Export(
        "trackDynamicDataAccessed",
    ): 6,
    Export(
        "trackDynamicFetch",
    ): 7,
    Export(
        "createPostponedAbortSignal",
    ): 2,
    Export(
        "formatDynamicAPIAccesses",
    ): 4,
}
```


# Modules (dev)
## Part 0
```js
"module evaluation";
import 'react';
import '../../client/components/hooks-server-context';
import '../../client/components/static-generation-bailout';
import '../../lib/url';

```
## Part 1
```js
import { postponeWithTracking } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 10
};
export { Postpone };
function Postpone({ reason, prerenderState, pathname }) {
    postponeWithTracking(prerenderState, reason, pathname);
}
export { Postpone } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 2
```js
import { assertPostpone } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 11
};
import { React } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 9
};
export { createPostponedAbortSignal };
function createPostponedAbortSignal(reason) {
    assertPostpone();
    const controller = new AbortController();
    try {
        React.unstable_postpone(reason);
    } catch (x) {
        controller.abort(x);
    }
    return controller.signal;
}
export { createPostponedAbortSignal } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 3
```js
export { createPrerenderState };
function createPrerenderState(isDebugSkeleton) {
    return {
        isDebugSkeleton,
        dynamicAccesses: []
    };
}
export { createPrerenderState } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 4
```js
export { formatDynamicAPIAccesses };
function formatDynamicAPIAccesses(prerenderState) {
    return prerenderState.dynamicAccesses.filter((access)=>typeof access.stack === 'string' && access.stack.length > 0).map(({ expression, stack })=>{
        stack = stack.split('\n').slice(4).filter((line)=>{
            if (line.includes('node_modules/next/')) {
                return false;
            }
            if (line.includes(' (<anonymous>)')) {
                return false;
            }
            if (line.includes(' (node:')) {
                return false;
            }
            return true;
        }).join('\n');
        return `Dynamic API Usage Debug - ${expression}:\n${stack}`;
    });
}
export { formatDynamicAPIAccesses } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 5
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 6
};
import { postponeWithTracking } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 10
};
export { markCurrentScopeAsDynamic };
import { DynamicServerError } from '../../client/components/hooks-server-context';
import { StaticGenBailoutError } from '../../client/components/static-generation-bailout';
import { getPathname } from '../../lib/url';
function markCurrentScopeAsDynamic(store, expression) {
    const pathname = getPathname(store.urlPathname);
    if (store.isUnstableCacheCallback) {
        return;
    } else if (store.dynamicShouldError) {
        throw new StaticGenBailoutError(`Route ${pathname} with \`dynamic = "error"\` couldn't be rendered statically because it used \`${expression}\`. See more info here: https://nextjs.org/docs/app/building-your-application/rendering/static-and-dynamic#dynamic-rendering`);
    } else if (store.prerenderState) {
        postponeWithTracking(store.prerenderState, expression, pathname);
    } else {
        store.revalidate = 0;
        if (store.isStaticGeneration) {
            const err = new DynamicServerError(`Route ${pathname} couldn't be rendered statically because it used ${expression}. See more info here: https://nextjs.org/docs/messages/dynamic-server-error`);
            store.dynamicUsageDescription = expression;
            store.dynamicUsageStack = err.stack;
            throw err;
        }
    }
}
export { DynamicServerError } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { StaticGenBailoutError } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { getPathname } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { markCurrentScopeAsDynamic } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 6
```js
import { postponeWithTracking } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 10
};
export { trackDynamicDataAccessed };
import { DynamicServerError } from '../../client/components/hooks-server-context';
import { StaticGenBailoutError } from '../../client/components/static-generation-bailout';
import { getPathname } from '../../lib/url';
function trackDynamicDataAccessed(store, expression) {
    const pathname = getPathname(store.urlPathname);
    if (store.isUnstableCacheCallback) {
        throw new Error(`Route ${pathname} used "${expression}" inside a function cached with "unstable_cache(...)". Accessing Dynamic data sources inside a cache scope is not supported. If you need this data inside a cached function use "${expression}" outside of the cached function and pass the required dynamic data in as an argument. See more info here: https://nextjs.org/docs/app/api-reference/functions/unstable_cache`);
    } else if (store.dynamicShouldError) {
        throw new StaticGenBailoutError(`Route ${pathname} with \`dynamic = "error"\` couldn't be rendered statically because it used \`${expression}\`. See more info here: https://nextjs.org/docs/app/building-your-application/rendering/static-and-dynamic#dynamic-rendering`);
    } else if (store.prerenderState) {
        postponeWithTracking(store.prerenderState, expression, pathname);
    } else {
        store.revalidate = 0;
        if (store.isStaticGeneration) {
            const err = new DynamicServerError(`Route ${pathname} couldn't be rendered statically because it used ${expression}. See more info here: https://nextjs.org/docs/messages/dynamic-server-error`);
            store.dynamicUsageDescription = expression;
            store.dynamicUsageStack = err.stack;
            throw err;
        }
    }
}
export { DynamicServerError } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { StaticGenBailoutError } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { getPathname } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { trackDynamicDataAccessed } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 7
```js
import { postponeWithTracking } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 10
};
export { trackDynamicFetch };
function trackDynamicFetch(store, expression) {
    if (!store.prerenderState || store.isUnstableCacheCallback) return;
    postponeWithTracking(store.prerenderState, expression, store.urlPathname);
}
export { trackDynamicFetch } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 8
```js
export { usedDynamicAPIs };
function usedDynamicAPIs(prerenderState) {
    return prerenderState.dynamicAccesses.length > 0;
}
export { usedDynamicAPIs } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 9
```js
import React from 'react';
const hasPostpone = typeof React.unstable_postpone === 'function';
export { React } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { hasPostpone } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 10
```js
import { assertPostpone } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 11
};
import { React } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 9
};
function postponeWithTracking(prerenderState, expression, pathname) {
    assertPostpone();
    const reason = `Route ${pathname} needs to bail out of prerendering at this point because it used ${expression}. ` + `React throws this special object to indicate where. It should not be caught by ` + `your own try/catch. Learn more: https://nextjs.org/docs/messages/ppr-caught-error`;
    prerenderState.dynamicAccesses.push({
        stack: prerenderState.isDebugSkeleton ? new Error().stack : undefined,
        expression
    });
    React.unstable_postpone(reason);
}
export { postponeWithTracking } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 11
```js
import { hasPostpone } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 9
};
function assertPostpone() {
    if (!hasPostpone) {
        throw new Error(`Invariant: React.unstable_postpone is not defined. This suggests the wrong version of React was loaded. This is a bug in Next.js`);
    }
}
export { assertPostpone } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Merged (module eval)
```js
import 'react';
import '../../client/components/hooks-server-context';
import '../../client/components/static-generation-bailout';
import '../../lib/url';
"module evaluation";

```
# Entrypoints

```
{
    ModuleEvaluation: 0,
    Export(
        "createPrerenderState",
    ): 3,
    Export(
        "markCurrentScopeAsDynamic",
    ): 5,
    Export(
        "usedDynamicAPIs",
    ): 8,
    Export(
        "Postpone",
    ): 1,
    Export(
        "trackDynamicDataAccessed",
    ): 6,
    Export(
        "trackDynamicFetch",
    ): 7,
    Export(
        "createPostponedAbortSignal",
    ): 2,
    Export(
        "formatDynamicAPIAccesses",
    ): 4,
}
```


# Modules (prod)
## Part 0
```js
"module evaluation";
import 'react';
import '../../client/components/hooks-server-context';
import '../../client/components/static-generation-bailout';
import '../../lib/url';

```
## Part 1
```js
import { postponeWithTracking } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 10
};
export { Postpone };
function Postpone({ reason, prerenderState, pathname }) {
    postponeWithTracking(prerenderState, reason, pathname);
}
export { Postpone } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 2
```js
import { assertPostpone } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 11
};
import { React } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 9
};
export { createPostponedAbortSignal };
function createPostponedAbortSignal(reason) {
    assertPostpone();
    const controller = new AbortController();
    try {
        React.unstable_postpone(reason);
    } catch (x) {
        controller.abort(x);
    }
    return controller.signal;
}
export { createPostponedAbortSignal } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 3
```js
export { createPrerenderState };
function createPrerenderState(isDebugSkeleton) {
    return {
        isDebugSkeleton,
        dynamicAccesses: []
    };
}
export { createPrerenderState } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 4
```js
export { formatDynamicAPIAccesses };
function formatDynamicAPIAccesses(prerenderState) {
    return prerenderState.dynamicAccesses.filter((access)=>typeof access.stack === 'string' && access.stack.length > 0).map(({ expression, stack })=>{
        stack = stack.split('\n').slice(4).filter((line)=>{
            if (line.includes('node_modules/next/')) {
                return false;
            }
            if (line.includes(' (<anonymous>)')) {
                return false;
            }
            if (line.includes(' (node:')) {
                return false;
            }
            return true;
        }).join('\n');
        return `Dynamic API Usage Debug - ${expression}:\n${stack}`;
    });
}
export { formatDynamicAPIAccesses } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 5
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 6
};
import { postponeWithTracking } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 10
};
export { markCurrentScopeAsDynamic };
import { DynamicServerError } from '../../client/components/hooks-server-context';
import { StaticGenBailoutError } from '../../client/components/static-generation-bailout';
import { getPathname } from '../../lib/url';
function markCurrentScopeAsDynamic(store, expression) {
    const pathname = getPathname(store.urlPathname);
    if (store.isUnstableCacheCallback) {
        return;
    } else if (store.dynamicShouldError) {
        throw new StaticGenBailoutError(`Route ${pathname} with \`dynamic = "error"\` couldn't be rendered statically because it used \`${expression}\`. See more info here: https://nextjs.org/docs/app/building-your-application/rendering/static-and-dynamic#dynamic-rendering`);
    } else if (store.prerenderState) {
        postponeWithTracking(store.prerenderState, expression, pathname);
    } else {
        store.revalidate = 0;
        if (store.isStaticGeneration) {
            const err = new DynamicServerError(`Route ${pathname} couldn't be rendered statically because it used ${expression}. See more info here: https://nextjs.org/docs/messages/dynamic-server-error`);
            store.dynamicUsageDescription = expression;
            store.dynamicUsageStack = err.stack;
            throw err;
        }
    }
}
export { DynamicServerError } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { StaticGenBailoutError } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { getPathname } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { markCurrentScopeAsDynamic } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 6
```js
import { postponeWithTracking } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 10
};
export { trackDynamicDataAccessed };
import { DynamicServerError } from '../../client/components/hooks-server-context';
import { StaticGenBailoutError } from '../../client/components/static-generation-bailout';
import { getPathname } from '../../lib/url';
function trackDynamicDataAccessed(store, expression) {
    const pathname = getPathname(store.urlPathname);
    if (store.isUnstableCacheCallback) {
        throw new Error(`Route ${pathname} used "${expression}" inside a function cached with "unstable_cache(...)". Accessing Dynamic data sources inside a cache scope is not supported. If you need this data inside a cached function use "${expression}" outside of the cached function and pass the required dynamic data in as an argument. See more info here: https://nextjs.org/docs/app/api-reference/functions/unstable_cache`);
    } else if (store.dynamicShouldError) {
        throw new StaticGenBailoutError(`Route ${pathname} with \`dynamic = "error"\` couldn't be rendered statically because it used \`${expression}\`. See more info here: https://nextjs.org/docs/app/building-your-application/rendering/static-and-dynamic#dynamic-rendering`);
    } else if (store.prerenderState) {
        postponeWithTracking(store.prerenderState, expression, pathname);
    } else {
        store.revalidate = 0;
        if (store.isStaticGeneration) {
            const err = new DynamicServerError(`Route ${pathname} couldn't be rendered statically because it used ${expression}. See more info here: https://nextjs.org/docs/messages/dynamic-server-error`);
            store.dynamicUsageDescription = expression;
            store.dynamicUsageStack = err.stack;
            throw err;
        }
    }
}
export { DynamicServerError } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { StaticGenBailoutError } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { getPathname } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { trackDynamicDataAccessed } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 7
```js
import { postponeWithTracking } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 10
};
export { trackDynamicFetch };
function trackDynamicFetch(store, expression) {
    if (!store.prerenderState || store.isUnstableCacheCallback) return;
    postponeWithTracking(store.prerenderState, expression, store.urlPathname);
}
export { trackDynamicFetch } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 8
```js
export { usedDynamicAPIs };
function usedDynamicAPIs(prerenderState) {
    return prerenderState.dynamicAccesses.length > 0;
}
export { usedDynamicAPIs } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 9
```js
import React from 'react';
const hasPostpone = typeof React.unstable_postpone === 'function';
export { React } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { hasPostpone } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 10
```js
import { assertPostpone } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 11
};
import { React } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 9
};
function postponeWithTracking(prerenderState, expression, pathname) {
    assertPostpone();
    const reason = `Route ${pathname} needs to bail out of prerendering at this point because it used ${expression}. ` + `React throws this special object to indicate where. It should not be caught by ` + `your own try/catch. Learn more: https://nextjs.org/docs/messages/ppr-caught-error`;
    prerenderState.dynamicAccesses.push({
        stack: prerenderState.isDebugSkeleton ? new Error().stack : undefined,
        expression
    });
    React.unstable_postpone(reason);
}
export { postponeWithTracking } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 11
```js
import { hasPostpone } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 9
};
function assertPostpone() {
    if (!hasPostpone) {
        throw new Error(`Invariant: React.unstable_postpone is not defined. This suggests the wrong version of React was loaded. This is a bug in Next.js`);
    }
}
export { assertPostpone } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Merged (module eval)
```js
import 'react';
import '../../client/components/hooks-server-context';
import '../../client/components/static-generation-bailout';
import '../../lib/url';
"module evaluation";

```
