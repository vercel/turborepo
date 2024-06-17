# Items

Count: 19

## Item 8: Stmt 0, `ImportOfModule`

```js
import { AppRouteRouteModule } from '../../server/future/route-modules/app-route/module.compiled';

```

- Hoisted
- Side effects

## Item 9: Stmt 0, `ImportBinding(0)`

```js
import { AppRouteRouteModule } from '../../server/future/route-modules/app-route/module.compiled';

```

- Hoisted
- Declares: `AppRouteRouteModule`

## Item 10: Stmt 1, `ImportOfModule`

```js
import { RouteKind } from '../../server/future/route-kind';

```

- Hoisted
- Side effects

## Item 11: Stmt 1, `ImportBinding(0)`

```js
import { RouteKind } from '../../server/future/route-kind';

```

- Hoisted
- Declares: `RouteKind`

## Item 12: Stmt 2, `ImportOfModule`

```js
import { patchFetch as _patchFetch } from '../../server/lib/patch-fetch';

```

- Hoisted
- Side effects

## Item 13: Stmt 2, `ImportBinding(0)`

```js
import { patchFetch as _patchFetch } from '../../server/lib/patch-fetch';

```

- Hoisted
- Declares: `_patchFetch`

## Item 14: Stmt 3, `ImportOfModule`

```js
import * as userland from 'VAR_USERLAND';

```

- Hoisted
- Side effects

## Item 15: Stmt 3, `ImportBinding(0)`

```js
import * as userland from 'VAR_USERLAND';

```

- Hoisted
- Declares: `userland`

## Item 16: Stmt 4, `VarDeclarator(0)`

```js
const routeModule = new AppRouteRouteModule({
    definition: {
        kind: RouteKind.APP_ROUTE,
        page: 'VAR_DEFINITION_PAGE',
        pathname: 'VAR_DEFINITION_PATHNAME',
        filename: 'VAR_DEFINITION_FILENAME',
        bundlePath: 'VAR_DEFINITION_BUNDLE_PATH'
    },
    resolvedPagePath: 'VAR_RESOLVED_PAGE_PATH',
    nextConfigOutput,
    userland
});

```

- Side effects
- Declares: `routeModule`
- Reads: `AppRouteRouteModule`, `RouteKind`, `userland`
- Write: `routeModule`, `RouteKind`

## Item 17: Stmt 5, `VarDeclarator(0)`

```js
const { requestAsyncStorage, staticGenerationAsyncStorage, serverHooks } = routeModule;

```

- Declares: `requestAsyncStorage`, `staticGenerationAsyncStorage`, `serverHooks`
- Reads: `routeModule`
- Write: `requestAsyncStorage`, `staticGenerationAsyncStorage`, `serverHooks`

## Item 18: Stmt 6, `VarDeclarator(0)`

```js
const originalPathname = 'VAR_ORIGINAL_PATHNAME';

```

- Declares: `originalPathname`
- Write: `originalPathname`

## Item 19: Stmt 7, `Normal`

```js
function patchFetch() {
    return _patchFetch({
        serverHooks,
        staticGenerationAsyncStorage
    });
}

```

- Hoisted
- Declares: `patchFetch`
- Reads (eventual): `_patchFetch`, `serverHooks`, `staticGenerationAsyncStorage`
- Write: `patchFetch`

# Phase 1
```mermaid
graph TD
    Item5;
    Item5["ModuleEvaluation"];
    Item6;
    Item6["export originalPathname"];
    Item7;
    Item7["export patchFetch"];
    Item8;
    Item8["export requestAsyncStorage"];
    Item9;
    Item9["export routeModule"];
    Item10;
    Item10["export serverHooks"];
    Item11;
    Item11["export staticGenerationAsyncStorage"];
    Item1;
    Item12;
    Item2;
    Item13;
    Item3;
    Item14;
    Item4;
    Item15;
    Item16;
    Item17;
    Item18;
    Item19;
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
    Item6["export originalPathname"];
    Item7;
    Item7["export patchFetch"];
    Item8;
    Item8["export requestAsyncStorage"];
    Item9;
    Item9["export routeModule"];
    Item10;
    Item10["export serverHooks"];
    Item11;
    Item11["export staticGenerationAsyncStorage"];
    Item1;
    Item12;
    Item2;
    Item13;
    Item3;
    Item14;
    Item4;
    Item15;
    Item16;
    Item17;
    Item18;
    Item19;
    Item2 --> Item1;
    Item3 --> Item1;
    Item3 --> Item2;
    Item4 --> Item1;
    Item4 --> Item2;
    Item4 --> Item3;
    Item7 --> Item19;
    Item16 --> Item12;
    Item16 --> Item13;
    Item16 --> Item15;
    Item16 -.-> Item9;
    Item16 --> Item1;
    Item16 --> Item2;
    Item16 --> Item3;
    Item16 --> Item4;
    Item16 -.-> Item14;
    Item16 -.-> Item10;
    Item16 -.-> Item11;
    Item17 --> Item16;
    Item17 -.-> Item8;
    Item17 -.-> Item11;
    Item17 -.-> Item10;
    Item18 -.-> Item6;
```
# Phase 3
```mermaid
graph TD
    Item5;
    Item5["ModuleEvaluation"];
    Item6;
    Item6["export originalPathname"];
    Item7;
    Item7["export patchFetch"];
    Item8;
    Item8["export requestAsyncStorage"];
    Item9;
    Item9["export routeModule"];
    Item10;
    Item10["export serverHooks"];
    Item11;
    Item11["export staticGenerationAsyncStorage"];
    Item1;
    Item12;
    Item2;
    Item13;
    Item3;
    Item14;
    Item4;
    Item15;
    Item16;
    Item17;
    Item18;
    Item19;
    Item2 --> Item1;
    Item3 --> Item1;
    Item3 --> Item2;
    Item4 --> Item1;
    Item4 --> Item2;
    Item4 --> Item3;
    Item7 --> Item19;
    Item16 --> Item12;
    Item16 --> Item13;
    Item16 --> Item15;
    Item16 -.-> Item9;
    Item16 --> Item1;
    Item16 --> Item2;
    Item16 --> Item3;
    Item16 --> Item4;
    Item16 -.-> Item14;
    Item16 -.-> Item10;
    Item16 -.-> Item11;
    Item17 --> Item16;
    Item17 -.-> Item8;
    Item17 -.-> Item11;
    Item17 -.-> Item10;
    Item18 -.-> Item6;
    Item19 --> Item14;
    Item19 --> Item17;
```
# Phase 4
```mermaid
graph TD
    Item5;
    Item5["ModuleEvaluation"];
    Item6;
    Item6["export originalPathname"];
    Item7;
    Item7["export patchFetch"];
    Item8;
    Item8["export requestAsyncStorage"];
    Item9;
    Item9["export routeModule"];
    Item10;
    Item10["export serverHooks"];
    Item11;
    Item11["export staticGenerationAsyncStorage"];
    Item1;
    Item12;
    Item2;
    Item13;
    Item3;
    Item14;
    Item4;
    Item15;
    Item16;
    Item17;
    Item18;
    Item19;
    Item2 --> Item1;
    Item3 --> Item1;
    Item3 --> Item2;
    Item4 --> Item1;
    Item4 --> Item2;
    Item4 --> Item3;
    Item7 --> Item19;
    Item16 --> Item12;
    Item16 --> Item13;
    Item16 --> Item15;
    Item16 -.-> Item9;
    Item16 --> Item1;
    Item16 --> Item2;
    Item16 --> Item3;
    Item16 --> Item4;
    Item16 -.-> Item14;
    Item16 -.-> Item10;
    Item16 -.-> Item11;
    Item17 --> Item16;
    Item17 -.-> Item8;
    Item17 -.-> Item11;
    Item17 -.-> Item10;
    Item18 -.-> Item6;
    Item19 --> Item14;
    Item19 --> Item17;
    Item5 --> Item1;
    Item5 --> Item2;
    Item5 --> Item3;
    Item5 --> Item4;
    Item5 --> Item16;
    Item6 --> Item18;
    Item8 --> Item17;
    Item9 --> Item16;
    Item10 --> Item17;
    Item11 --> Item17;
```
# Final
```mermaid
graph TD
    N0["Items: [ItemId(ModuleEvaluation)]"];
    N1["Items: [ItemId(Export((&quot;originalPathname&quot;, #2), &quot;originalPathname&quot;)), ItemId(6, VarDeclarator(0))]"];
    N2["Items: [ItemId(Export((&quot;patchFetch&quot;, #2), &quot;patchFetch&quot;)), ItemId(2, ImportBinding(0)), ItemId(7, Normal)]"];
    N3["Items: [ItemId(Export((&quot;requestAsyncStorage&quot;, #2), &quot;requestAsyncStorage&quot;))]"];
    N4["Items: [ItemId(Export((&quot;routeModule&quot;, #2), &quot;routeModule&quot;))]"];
    N5["Items: [ItemId(Export((&quot;serverHooks&quot;, #2), &quot;serverHooks&quot;))]"];
    N6["Items: [ItemId(Export((&quot;staticGenerationAsyncStorage&quot;, #2), &quot;staticGenerationAsyncStorage&quot;))]"];
    N7["Items: [ItemId(0, ImportOfModule)]"];
    N8["Items: [ItemId(1, ImportOfModule)]"];
    N9["Items: [ItemId(2, ImportOfModule)]"];
    N10["Items: [ItemId(3, ImportOfModule)]"];
    N11["Items: [ItemId(0, ImportBinding(0)), ItemId(1, ImportBinding(0)), ItemId(2, ImportBinding(0)), ItemId(3, ImportBinding(0)), ItemId(4, VarDeclarator(0))]"];
    N12["Items: [ItemId(5, VarDeclarator(0))]"];
    N0 --> N7;
    N0 --> N8;
    N0 --> N9;
    N0 --> N10;
    N0 --> N11;
    N2 --> N11;
    N2 --> N12;
    N3 --> N12;
    N4 --> N11;
    N5 --> N12;
    N6 --> N12;
    N8 --> N7;
    N9 --> N7;
    N9 --> N8;
    N10 --> N7;
    N10 --> N8;
    N10 --> N9;
    N11 --> N4;
    N11 --> N7;
    N11 --> N8;
    N11 --> N9;
    N11 --> N10;
    N11 --> N5;
    N11 --> N6;
    N12 --> N11;
    N12 --> N3;
    N12 --> N6;
    N12 --> N5;
```
# Entrypoints

```
{
    ModuleEvaluation: 0,
    Export(
        "patchFetch",
    ): 2,
    Export(
        "requestAsyncStorage",
    ): 3,
    Export(
        "serverHooks",
    ): 5,
    Export(
        "staticGenerationAsyncStorage",
    ): 6,
    Export(
        "originalPathname",
    ): 1,
    Export(
        "routeModule",
    ): 4,
}
```


# Modules (dev)
## Part 0
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 7
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 8
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 9
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 10
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 11
};
"module evaluation";

```
## Part 1
```js
export { originalPathname as originalPathname };
const originalPathname = 'VAR_ORIGINAL_PATHNAME';
export { originalPathname } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 2
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 11
};
import { staticGenerationAsyncStorage, serverHooks } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 12
};
export { patchFetch as patchFetch };
import { patchFetch as _patchFetch } from '../../server/lib/patch-fetch';
function patchFetch() {
    return _patchFetch({
        serverHooks,
        staticGenerationAsyncStorage
    });
}
export { _patchFetch } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { patchFetch } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 3
```js
import { requestAsyncStorage } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 12
};
export { requestAsyncStorage as requestAsyncStorage };

```
## Part 4
```js
import { routeModule } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 11
};
export { routeModule as routeModule };

```
## Part 5
```js
import { serverHooks } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 12
};
export { serverHooks as serverHooks };

```
## Part 6
```js
import { staticGenerationAsyncStorage } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 12
};
export { staticGenerationAsyncStorage as staticGenerationAsyncStorage };

```
## Part 7
```js
import '../../server/future/route-modules/app-route/module.compiled';

```
## Part 8
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 7
};
import '../../server/future/route-kind';

```
## Part 9
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 7
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 8
};
import '../../server/lib/patch-fetch';

```
## Part 10
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 7
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 8
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 9
};
import 'VAR_USERLAND';

```
## Part 11
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 4
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 7
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 8
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 9
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 10
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 6
};
import { AppRouteRouteModule } from '../../server/future/route-modules/app-route/module.compiled';
import { RouteKind } from '../../server/future/route-kind';
import { patchFetch as _patchFetch } from '../../server/lib/patch-fetch';
import * as userland from 'VAR_USERLAND';
const routeModule = new AppRouteRouteModule({
    definition: {
        kind: RouteKind.APP_ROUTE,
        page: 'VAR_DEFINITION_PAGE',
        pathname: 'VAR_DEFINITION_PATHNAME',
        filename: 'VAR_DEFINITION_FILENAME',
        bundlePath: 'VAR_DEFINITION_BUNDLE_PATH'
    },
    resolvedPagePath: 'VAR_RESOLVED_PAGE_PATH',
    nextConfigOutput,
    userland
});
export { AppRouteRouteModule } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { RouteKind } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { _patchFetch } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { userland } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { routeModule } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 12
```js
import { routeModule } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 11
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 3
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 6
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
const { requestAsyncStorage, staticGenerationAsyncStorage, serverHooks } = routeModule;
export { requestAsyncStorage } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { staticGenerationAsyncStorage } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { serverHooks } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Merged (module eval)
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 7
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 8
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 9
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 10
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 11
};
"module evaluation";

```
# Entrypoints

```
{
    ModuleEvaluation: 0,
    Export(
        "patchFetch",
    ): 2,
    Export(
        "requestAsyncStorage",
    ): 3,
    Export(
        "serverHooks",
    ): 5,
    Export(
        "staticGenerationAsyncStorage",
    ): 6,
    Export(
        "originalPathname",
    ): 1,
    Export(
        "routeModule",
    ): 4,
}
```


# Modules (prod)
## Part 0
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 7
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 8
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 9
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 10
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 11
};
"module evaluation";

```
## Part 1
```js
export { originalPathname as originalPathname };
const originalPathname = 'VAR_ORIGINAL_PATHNAME';
export { originalPathname } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 2
```js
import { staticGenerationAsyncStorage, serverHooks } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 12
};
export { patchFetch as patchFetch };
import { patchFetch as _patchFetch } from '../../server/lib/patch-fetch';
function patchFetch() {
    return _patchFetch({
        serverHooks,
        staticGenerationAsyncStorage
    });
}
export { _patchFetch } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { patchFetch } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 3
```js
import { requestAsyncStorage } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 12
};
export { requestAsyncStorage as requestAsyncStorage };

```
## Part 4
```js
import { routeModule } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 11
};
export { routeModule as routeModule };

```
## Part 5
```js
import { serverHooks } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 12
};
export { serverHooks as serverHooks };

```
## Part 6
```js
import { staticGenerationAsyncStorage } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 12
};
export { staticGenerationAsyncStorage as staticGenerationAsyncStorage };

```
## Part 7
```js
import '../../server/future/route-modules/app-route/module.compiled';

```
## Part 8
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 7
};
import '../../server/future/route-kind';

```
## Part 9
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 7
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 8
};
import '../../server/lib/patch-fetch';

```
## Part 10
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 7
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 8
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 9
};
import 'VAR_USERLAND';

```
## Part 11
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 7
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 8
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 9
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 10
};
import { AppRouteRouteModule } from '../../server/future/route-modules/app-route/module.compiled';
import { RouteKind } from '../../server/future/route-kind';
import * as userland from 'VAR_USERLAND';
const routeModule = new AppRouteRouteModule({
    definition: {
        kind: RouteKind.APP_ROUTE,
        page: 'VAR_DEFINITION_PAGE',
        pathname: 'VAR_DEFINITION_PATHNAME',
        filename: 'VAR_DEFINITION_FILENAME',
        bundlePath: 'VAR_DEFINITION_BUNDLE_PATH'
    },
    resolvedPagePath: 'VAR_RESOLVED_PAGE_PATH',
    nextConfigOutput,
    userland
});
export { AppRouteRouteModule } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { RouteKind } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { userland } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { routeModule } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 12
```js
import { routeModule } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 11
};
const { requestAsyncStorage, staticGenerationAsyncStorage, serverHooks } = routeModule;
export { requestAsyncStorage } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { staticGenerationAsyncStorage } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { serverHooks } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Merged (module eval)
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 7
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 8
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 9
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 10
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 11
};
"module evaluation";

```
