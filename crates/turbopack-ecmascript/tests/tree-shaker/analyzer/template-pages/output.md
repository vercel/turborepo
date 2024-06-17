# Items

Count: 37

## Item 14: Stmt 0, `ImportOfModule`

```js
import { PagesRouteModule } from '../../server/future/route-modules/pages/module.compiled';

```

- Hoisted
- Side effects

## Item 15: Stmt 0, `ImportBinding(0)`

```js
import { PagesRouteModule } from '../../server/future/route-modules/pages/module.compiled';

```

- Hoisted
- Declares: `PagesRouteModule`

## Item 16: Stmt 1, `ImportOfModule`

```js
import { RouteKind } from '../../server/future/route-kind';

```

- Hoisted
- Side effects

## Item 17: Stmt 1, `ImportBinding(0)`

```js
import { RouteKind } from '../../server/future/route-kind';

```

- Hoisted
- Declares: `RouteKind`

## Item 18: Stmt 2, `ImportOfModule`

```js
import { hoist } from './helpers';

```

- Hoisted
- Side effects

## Item 19: Stmt 2, `ImportBinding(0)`

```js
import { hoist } from './helpers';

```

- Hoisted
- Declares: `hoist`

## Item 20: Stmt 3, `ImportOfModule`

```js
import Document from 'VAR_MODULE_DOCUMENT';

```

- Hoisted
- Side effects

## Item 21: Stmt 3, `ImportBinding(0)`

```js
import Document from 'VAR_MODULE_DOCUMENT';

```

- Hoisted
- Declares: `Document`

## Item 22: Stmt 4, `ImportOfModule`

```js
import App from 'VAR_MODULE_APP';

```

- Hoisted
- Side effects

## Item 23: Stmt 4, `ImportBinding(0)`

```js
import App from 'VAR_MODULE_APP';

```

- Hoisted
- Declares: `App`

## Item 24: Stmt 5, `ImportOfModule`

```js
import * as userland from 'VAR_USERLAND';

```

- Hoisted
- Side effects

## Item 25: Stmt 5, `ImportBinding(0)`

```js
import * as userland from 'VAR_USERLAND';

```

- Hoisted
- Declares: `userland`

## Item 26: Stmt 6, `Normal`

```js
export default hoist(userland, 'default');

```

- Side effects
- Declares: `__TURBOPACK__default__export__`
- Reads: `hoist`, `userland`
- Write: `__TURBOPACK__default__export__`

## Item 27: Stmt 7, `VarDeclarator(0)`

```js
export const getStaticProps = hoist(userland, 'getStaticProps');

```

- Declares: `getStaticProps`
- Reads: `hoist`, `userland`
- Write: `getStaticProps`

## Item 28: Stmt 8, `VarDeclarator(0)`

```js
export const getStaticPaths = hoist(userland, 'getStaticPaths');

```

- Declares: `getStaticPaths`
- Reads: `hoist`, `userland`
- Write: `getStaticPaths`

## Item 29: Stmt 9, `VarDeclarator(0)`

```js
export const getServerSideProps = hoist(userland, 'getServerSideProps');

```

- Declares: `getServerSideProps`
- Reads: `hoist`, `userland`
- Write: `getServerSideProps`

## Item 30: Stmt 10, `VarDeclarator(0)`

```js
export const config = hoist(userland, 'config');

```

- Declares: `config`
- Reads: `hoist`, `userland`
- Write: `config`

## Item 31: Stmt 11, `VarDeclarator(0)`

```js
export const reportWebVitals = hoist(userland, 'reportWebVitals');

```

- Declares: `reportWebVitals`
- Reads: `hoist`, `userland`
- Write: `reportWebVitals`

## Item 32: Stmt 12, `VarDeclarator(0)`

```js
export const unstable_getStaticProps = hoist(userland, 'unstable_getStaticProps');

```

- Declares: `unstable_getStaticProps`
- Reads: `hoist`, `userland`
- Write: `unstable_getStaticProps`

## Item 33: Stmt 13, `VarDeclarator(0)`

```js
export const unstable_getStaticPaths = hoist(userland, 'unstable_getStaticPaths');

```

- Declares: `unstable_getStaticPaths`
- Reads: `hoist`, `userland`
- Write: `unstable_getStaticPaths`

## Item 34: Stmt 14, `VarDeclarator(0)`

```js
export const unstable_getStaticParams = hoist(userland, 'unstable_getStaticParams');

```

- Declares: `unstable_getStaticParams`
- Reads: `hoist`, `userland`
- Write: `unstable_getStaticParams`

## Item 35: Stmt 15, `VarDeclarator(0)`

```js
export const unstable_getServerProps = hoist(userland, 'unstable_getServerProps');

```

- Declares: `unstable_getServerProps`
- Reads: `hoist`, `userland`
- Write: `unstable_getServerProps`

## Item 36: Stmt 16, `VarDeclarator(0)`

```js
export const unstable_getServerSideProps = hoist(userland, 'unstable_getServerSideProps');

```

- Declares: `unstable_getServerSideProps`
- Reads: `hoist`, `userland`
- Write: `unstable_getServerSideProps`

## Item 37: Stmt 17, `VarDeclarator(0)`

```js
export const routeModule = new PagesRouteModule({
    definition: {
        kind: RouteKind.PAGES,
        page: 'VAR_DEFINITION_PAGE',
        pathname: 'VAR_DEFINITION_PATHNAME',
        bundlePath: '',
        filename: ''
    },
    components: {
        App,
        Document
    },
    userland
});

```

- Declares: `routeModule`
- Reads: `PagesRouteModule`, `RouteKind`, `App`, `Document`, `userland`
- Write: `routeModule`, `RouteKind`

# Phase 1
```mermaid
graph TD
    Item7;
    Item7["ModuleEvaluation"];
    Item8;
    Item8["export default"];
    Item9;
    Item9["export config"];
    Item10;
    Item10["export getServerSideProps"];
    Item11;
    Item11["export getStaticPaths"];
    Item12;
    Item12["export getStaticProps"];
    Item13;
    Item13["export reportWebVitals"];
    Item14;
    Item14["export routeModule"];
    Item15;
    Item15["export unstable_getServerProps"];
    Item16;
    Item16["export unstable_getServerSideProps"];
    Item17;
    Item17["export unstable_getStaticParams"];
    Item18;
    Item18["export unstable_getStaticPaths"];
    Item19;
    Item19["export unstable_getStaticProps"];
    Item1;
    Item20;
    Item2;
    Item21;
    Item3;
    Item22;
    Item4;
    Item23;
    Item5;
    Item24;
    Item6;
    Item25;
    Item26;
    Item27;
    Item28;
    Item29;
    Item30;
    Item31;
    Item32;
    Item33;
    Item34;
    Item35;
    Item36;
    Item37;
    Item2 --> Item1;
    Item3 --> Item1;
    Item3 --> Item2;
    Item4 --> Item1;
    Item4 --> Item2;
    Item4 --> Item3;
    Item5 --> Item1;
    Item5 --> Item2;
    Item5 --> Item3;
    Item5 --> Item4;
    Item6 --> Item1;
    Item6 --> Item2;
    Item6 --> Item3;
    Item6 --> Item4;
    Item6 --> Item5;
```
# Phase 2
```mermaid
graph TD
    Item7;
    Item7["ModuleEvaluation"];
    Item8;
    Item8["export default"];
    Item9;
    Item9["export config"];
    Item10;
    Item10["export getServerSideProps"];
    Item11;
    Item11["export getStaticPaths"];
    Item12;
    Item12["export getStaticProps"];
    Item13;
    Item13["export reportWebVitals"];
    Item14;
    Item14["export routeModule"];
    Item15;
    Item15["export unstable_getServerProps"];
    Item16;
    Item16["export unstable_getServerSideProps"];
    Item17;
    Item17["export unstable_getStaticParams"];
    Item18;
    Item18["export unstable_getStaticPaths"];
    Item19;
    Item19["export unstable_getStaticProps"];
    Item1;
    Item20;
    Item2;
    Item21;
    Item3;
    Item22;
    Item4;
    Item23;
    Item5;
    Item24;
    Item6;
    Item25;
    Item26;
    Item27;
    Item28;
    Item29;
    Item30;
    Item31;
    Item32;
    Item33;
    Item34;
    Item35;
    Item36;
    Item37;
    Item2 --> Item1;
    Item3 --> Item1;
    Item3 --> Item2;
    Item4 --> Item1;
    Item4 --> Item2;
    Item4 --> Item3;
    Item5 --> Item1;
    Item5 --> Item2;
    Item5 --> Item3;
    Item5 --> Item4;
    Item6 --> Item1;
    Item6 --> Item2;
    Item6 --> Item3;
    Item6 --> Item4;
    Item6 --> Item5;
    Item26 --> Item22;
    Item26 --> Item25;
    Item26 --> Item1;
    Item26 --> Item2;
    Item26 --> Item3;
    Item26 --> Item4;
    Item26 --> Item5;
    Item26 --> Item6;
    Item27 --> Item22;
    Item27 --> Item25;
    Item27 -.-> Item12;
    Item28 --> Item22;
    Item28 --> Item25;
    Item28 -.-> Item11;
    Item29 --> Item22;
    Item29 --> Item25;
    Item29 -.-> Item10;
    Item30 --> Item22;
    Item30 --> Item25;
    Item30 -.-> Item9;
    Item31 --> Item22;
    Item31 --> Item25;
    Item31 -.-> Item13;
    Item32 --> Item22;
    Item32 --> Item25;
    Item32 -.-> Item19;
    Item33 --> Item22;
    Item33 --> Item25;
    Item33 -.-> Item18;
    Item34 --> Item22;
    Item34 --> Item25;
    Item34 -.-> Item17;
    Item35 --> Item22;
    Item35 --> Item25;
    Item35 -.-> Item15;
    Item36 --> Item22;
    Item36 --> Item25;
    Item36 -.-> Item16;
    Item37 --> Item20;
    Item37 --> Item21;
    Item37 --> Item24;
    Item37 --> Item23;
    Item37 --> Item25;
    Item37 -.-> Item14;
```
# Phase 3
```mermaid
graph TD
    Item7;
    Item7["ModuleEvaluation"];
    Item8;
    Item8["export default"];
    Item9;
    Item9["export config"];
    Item10;
    Item10["export getServerSideProps"];
    Item11;
    Item11["export getStaticPaths"];
    Item12;
    Item12["export getStaticProps"];
    Item13;
    Item13["export reportWebVitals"];
    Item14;
    Item14["export routeModule"];
    Item15;
    Item15["export unstable_getServerProps"];
    Item16;
    Item16["export unstable_getServerSideProps"];
    Item17;
    Item17["export unstable_getStaticParams"];
    Item18;
    Item18["export unstable_getStaticPaths"];
    Item19;
    Item19["export unstable_getStaticProps"];
    Item1;
    Item20;
    Item2;
    Item21;
    Item3;
    Item22;
    Item4;
    Item23;
    Item5;
    Item24;
    Item6;
    Item25;
    Item26;
    Item27;
    Item28;
    Item29;
    Item30;
    Item31;
    Item32;
    Item33;
    Item34;
    Item35;
    Item36;
    Item37;
    Item2 --> Item1;
    Item3 --> Item1;
    Item3 --> Item2;
    Item4 --> Item1;
    Item4 --> Item2;
    Item4 --> Item3;
    Item5 --> Item1;
    Item5 --> Item2;
    Item5 --> Item3;
    Item5 --> Item4;
    Item6 --> Item1;
    Item6 --> Item2;
    Item6 --> Item3;
    Item6 --> Item4;
    Item6 --> Item5;
    Item26 --> Item22;
    Item26 --> Item25;
    Item26 --> Item1;
    Item26 --> Item2;
    Item26 --> Item3;
    Item26 --> Item4;
    Item26 --> Item5;
    Item26 --> Item6;
    Item27 --> Item22;
    Item27 --> Item25;
    Item27 -.-> Item12;
    Item28 --> Item22;
    Item28 --> Item25;
    Item28 -.-> Item11;
    Item29 --> Item22;
    Item29 --> Item25;
    Item29 -.-> Item10;
    Item30 --> Item22;
    Item30 --> Item25;
    Item30 -.-> Item9;
    Item31 --> Item22;
    Item31 --> Item25;
    Item31 -.-> Item13;
    Item32 --> Item22;
    Item32 --> Item25;
    Item32 -.-> Item19;
    Item33 --> Item22;
    Item33 --> Item25;
    Item33 -.-> Item18;
    Item34 --> Item22;
    Item34 --> Item25;
    Item34 -.-> Item17;
    Item35 --> Item22;
    Item35 --> Item25;
    Item35 -.-> Item15;
    Item36 --> Item22;
    Item36 --> Item25;
    Item36 -.-> Item16;
    Item37 --> Item20;
    Item37 --> Item21;
    Item37 --> Item24;
    Item37 --> Item23;
    Item37 --> Item25;
    Item37 -.-> Item14;
```
# Phase 4
```mermaid
graph TD
    Item7;
    Item7["ModuleEvaluation"];
    Item8;
    Item8["export default"];
    Item9;
    Item9["export config"];
    Item10;
    Item10["export getServerSideProps"];
    Item11;
    Item11["export getStaticPaths"];
    Item12;
    Item12["export getStaticProps"];
    Item13;
    Item13["export reportWebVitals"];
    Item14;
    Item14["export routeModule"];
    Item15;
    Item15["export unstable_getServerProps"];
    Item16;
    Item16["export unstable_getServerSideProps"];
    Item17;
    Item17["export unstable_getStaticParams"];
    Item18;
    Item18["export unstable_getStaticPaths"];
    Item19;
    Item19["export unstable_getStaticProps"];
    Item1;
    Item20;
    Item2;
    Item21;
    Item3;
    Item22;
    Item4;
    Item23;
    Item5;
    Item24;
    Item6;
    Item25;
    Item26;
    Item27;
    Item28;
    Item29;
    Item30;
    Item31;
    Item32;
    Item33;
    Item34;
    Item35;
    Item36;
    Item37;
    Item2 --> Item1;
    Item3 --> Item1;
    Item3 --> Item2;
    Item4 --> Item1;
    Item4 --> Item2;
    Item4 --> Item3;
    Item5 --> Item1;
    Item5 --> Item2;
    Item5 --> Item3;
    Item5 --> Item4;
    Item6 --> Item1;
    Item6 --> Item2;
    Item6 --> Item3;
    Item6 --> Item4;
    Item6 --> Item5;
    Item26 --> Item22;
    Item26 --> Item25;
    Item26 --> Item1;
    Item26 --> Item2;
    Item26 --> Item3;
    Item26 --> Item4;
    Item26 --> Item5;
    Item26 --> Item6;
    Item27 --> Item22;
    Item27 --> Item25;
    Item27 -.-> Item12;
    Item28 --> Item22;
    Item28 --> Item25;
    Item28 -.-> Item11;
    Item29 --> Item22;
    Item29 --> Item25;
    Item29 -.-> Item10;
    Item30 --> Item22;
    Item30 --> Item25;
    Item30 -.-> Item9;
    Item31 --> Item22;
    Item31 --> Item25;
    Item31 -.-> Item13;
    Item32 --> Item22;
    Item32 --> Item25;
    Item32 -.-> Item19;
    Item33 --> Item22;
    Item33 --> Item25;
    Item33 -.-> Item18;
    Item34 --> Item22;
    Item34 --> Item25;
    Item34 -.-> Item17;
    Item35 --> Item22;
    Item35 --> Item25;
    Item35 -.-> Item15;
    Item36 --> Item22;
    Item36 --> Item25;
    Item36 -.-> Item16;
    Item37 --> Item20;
    Item37 --> Item21;
    Item37 --> Item24;
    Item37 --> Item23;
    Item37 --> Item25;
    Item37 -.-> Item14;
    Item7 --> Item1;
    Item7 --> Item2;
    Item7 --> Item3;
    Item7 --> Item4;
    Item7 --> Item5;
    Item7 --> Item6;
    Item7 --> Item26;
    Item8 --> Item26;
    Item9 --> Item30;
    Item10 --> Item29;
    Item11 --> Item28;
    Item12 --> Item27;
    Item13 --> Item31;
    Item14 --> Item37;
    Item15 --> Item35;
    Item16 --> Item36;
    Item17 --> Item34;
    Item18 --> Item33;
    Item19 --> Item32;
```
# Final
```mermaid
graph TD
    N0["Items: [ItemId(ModuleEvaluation)]"];
    N1["Items: [ItemId(Export((&quot;__TURBOPACK__default__export__&quot;, #3), &quot;default&quot;))]"];
    N2["Items: [ItemId(Export((&quot;config&quot;, #2), &quot;config&quot;)), ItemId(2, ImportBinding(0)), ItemId(5, ImportBinding(0)), ItemId(10, VarDeclarator(0))]"];
    N3["Items: [ItemId(Export((&quot;getServerSideProps&quot;, #2), &quot;getServerSideProps&quot;)), ItemId(2, ImportBinding(0)), ItemId(5, ImportBinding(0)), ItemId(9, VarDeclarator(0))]"];
    N4["Items: [ItemId(Export((&quot;getStaticPaths&quot;, #2), &quot;getStaticPaths&quot;)), ItemId(2, ImportBinding(0)), ItemId(5, ImportBinding(0)), ItemId(8, VarDeclarator(0))]"];
    N5["Items: [ItemId(Export((&quot;getStaticProps&quot;, #2), &quot;getStaticProps&quot;)), ItemId(2, ImportBinding(0)), ItemId(5, ImportBinding(0)), ItemId(7, VarDeclarator(0))]"];
    N6["Items: [ItemId(Export((&quot;reportWebVitals&quot;, #2), &quot;reportWebVitals&quot;)), ItemId(2, ImportBinding(0)), ItemId(5, ImportBinding(0)), ItemId(11, VarDeclarator(0))]"];
    N7["Items: [ItemId(Export((&quot;routeModule&quot;, #2), &quot;routeModule&quot;)), ItemId(0, ImportBinding(0)), ItemId(1, ImportBinding(0)), ItemId(3, ImportBinding(0)), ItemId(4, ImportBinding(0)), ItemId(5, ImportBinding(0)), ItemId(17, VarDeclarator(0))]"];
    N8["Items: [ItemId(Export((&quot;unstable_getServerProps&quot;, #2), &quot;unstable_getServerProps&quot;)), ItemId(2, ImportBinding(0)), ItemId(5, ImportBinding(0)), ItemId(15, VarDeclarator(0))]"];
    N9["Items: [ItemId(Export((&quot;unstable_getServerSideProps&quot;, #2), &quot;unstable_getServerSideProps&quot;)), ItemId(2, ImportBinding(0)), ItemId(5, ImportBinding(0)), ItemId(16, VarDeclarator(0))]"];
    N10["Items: [ItemId(Export((&quot;unstable_getStaticParams&quot;, #2), &quot;unstable_getStaticParams&quot;)), ItemId(2, ImportBinding(0)), ItemId(5, ImportBinding(0)), ItemId(14, VarDeclarator(0))]"];
    N11["Items: [ItemId(Export((&quot;unstable_getStaticPaths&quot;, #2), &quot;unstable_getStaticPaths&quot;)), ItemId(2, ImportBinding(0)), ItemId(5, ImportBinding(0)), ItemId(13, VarDeclarator(0))]"];
    N12["Items: [ItemId(Export((&quot;unstable_getStaticProps&quot;, #2), &quot;unstable_getStaticProps&quot;)), ItemId(2, ImportBinding(0)), ItemId(5, ImportBinding(0)), ItemId(12, VarDeclarator(0))]"];
    N13["Items: [ItemId(0, ImportOfModule)]"];
    N14["Items: [ItemId(1, ImportOfModule)]"];
    N15["Items: [ItemId(2, ImportOfModule)]"];
    N16["Items: [ItemId(3, ImportOfModule)]"];
    N17["Items: [ItemId(4, ImportOfModule)]"];
    N18["Items: [ItemId(5, ImportOfModule)]"];
    N19["Items: [ItemId(2, ImportBinding(0)), ItemId(5, ImportBinding(0)), ItemId(6, Normal)]"];
    N0 --> N13;
    N0 --> N14;
    N0 --> N15;
    N0 --> N16;
    N0 --> N17;
    N0 --> N18;
    N0 --> N19;
    N1 --> N19;
    N2 --> N19;
    N3 --> N19;
    N4 --> N19;
    N5 --> N19;
    N6 --> N19;
    N7 --> N19;
    N8 --> N19;
    N9 --> N19;
    N10 --> N19;
    N11 --> N19;
    N12 --> N19;
    N14 --> N13;
    N15 --> N13;
    N15 --> N14;
    N16 --> N13;
    N16 --> N14;
    N16 --> N15;
    N17 --> N13;
    N17 --> N14;
    N17 --> N15;
    N17 --> N16;
    N18 --> N13;
    N18 --> N14;
    N18 --> N15;
    N18 --> N16;
    N18 --> N17;
    N19 --> N13;
    N19 --> N14;
    N19 --> N15;
    N19 --> N16;
    N19 --> N17;
    N19 --> N18;
```
# Entrypoints

```
{
    ModuleEvaluation: 0,
    Export(
        "unstable_getServerSideProps",
    ): 9,
    Export(
        "unstable_getStaticPaths",
    ): 11,
    Export(
        "reportWebVitals",
    ): 6,
    Export(
        "unstable_getServerProps",
    ): 8,
    Export(
        "routeModule",
    ): 7,
    Export(
        "getStaticProps",
    ): 5,
    Export(
        "config",
    ): 2,
    Export(
        "unstable_getStaticParams",
    ): 10,
    Export(
        "unstable_getStaticProps",
    ): 12,
    Export(
        "default",
    ): 1,
    Export(
        "getServerSideProps",
    ): 3,
    Export(
        "getStaticPaths",
    ): 4,
}
```


# Modules (dev)
## Part 0
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 13
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 14
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 15
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 16
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 17
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 18
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 19
};
"module evaluation";

```
## Part 1
```js
import { __TURBOPACK__default__export__ } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 19
};
export { __TURBOPACK__default__export__ as default };

```
## Part 2
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 19
};
export { config };
import { hoist } from './helpers';
import * as userland from 'VAR_USERLAND';
const config = hoist(userland, 'config');
export { hoist } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { userland } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { config } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 3
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 19
};
export { getServerSideProps };
import { hoist } from './helpers';
import * as userland from 'VAR_USERLAND';
const getServerSideProps = hoist(userland, 'getServerSideProps');
export { hoist } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { userland } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { getServerSideProps } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 4
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 19
};
export { getStaticPaths };
import { hoist } from './helpers';
import * as userland from 'VAR_USERLAND';
const getStaticPaths = hoist(userland, 'getStaticPaths');
export { hoist } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { userland } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { getStaticPaths } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 5
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 19
};
export { getStaticProps };
import { hoist } from './helpers';
import * as userland from 'VAR_USERLAND';
const getStaticProps = hoist(userland, 'getStaticProps');
export { hoist } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { userland } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { getStaticProps } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 6
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 19
};
export { reportWebVitals };
import { hoist } from './helpers';
import * as userland from 'VAR_USERLAND';
const reportWebVitals = hoist(userland, 'reportWebVitals');
export { hoist } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { userland } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { reportWebVitals } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 7
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 19
};
export { routeModule };
import { PagesRouteModule } from '../../server/future/route-modules/pages/module.compiled';
import { RouteKind } from '../../server/future/route-kind';
import Document from 'VAR_MODULE_DOCUMENT';
import App from 'VAR_MODULE_APP';
import * as userland from 'VAR_USERLAND';
const routeModule = new PagesRouteModule({
    definition: {
        kind: RouteKind.PAGES,
        page: 'VAR_DEFINITION_PAGE',
        pathname: 'VAR_DEFINITION_PATHNAME',
        bundlePath: '',
        filename: ''
    },
    components: {
        App,
        Document
    },
    userland
});
export { PagesRouteModule } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { RouteKind } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { Document } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { App } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { userland } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { routeModule } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 8
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 19
};
export { unstable_getServerProps };
import { hoist } from './helpers';
import * as userland from 'VAR_USERLAND';
const unstable_getServerProps = hoist(userland, 'unstable_getServerProps');
export { hoist } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { userland } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { unstable_getServerProps } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 9
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 19
};
export { unstable_getServerSideProps };
import { hoist } from './helpers';
import * as userland from 'VAR_USERLAND';
const unstable_getServerSideProps = hoist(userland, 'unstable_getServerSideProps');
export { hoist } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { userland } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { unstable_getServerSideProps } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 10
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 19
};
export { unstable_getStaticParams };
import { hoist } from './helpers';
import * as userland from 'VAR_USERLAND';
const unstable_getStaticParams = hoist(userland, 'unstable_getStaticParams');
export { hoist } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { userland } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { unstable_getStaticParams } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 11
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 19
};
export { unstable_getStaticPaths };
import { hoist } from './helpers';
import * as userland from 'VAR_USERLAND';
const unstable_getStaticPaths = hoist(userland, 'unstable_getStaticPaths');
export { hoist } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { userland } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { unstable_getStaticPaths } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 12
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 19
};
export { unstable_getStaticProps };
import { hoist } from './helpers';
import * as userland from 'VAR_USERLAND';
const unstable_getStaticProps = hoist(userland, 'unstable_getStaticProps');
export { hoist } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { userland } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { unstable_getStaticProps } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 13
```js
import '../../server/future/route-modules/pages/module.compiled';

```
## Part 14
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 13
};
import '../../server/future/route-kind';

```
## Part 15
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 13
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 14
};
import './helpers';

```
## Part 16
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 13
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 14
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 15
};
import 'VAR_MODULE_DOCUMENT';

```
## Part 17
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 13
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 14
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 15
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 16
};
import 'VAR_MODULE_APP';

```
## Part 18
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 13
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 14
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 15
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 16
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 17
};
import 'VAR_USERLAND';

```
## Part 19
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 13
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 14
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 15
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 16
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 17
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 18
};
import { hoist } from './helpers';
import * as userland from 'VAR_USERLAND';
const __TURBOPACK__default__export__ = hoist(userland, 'default');
export { hoist } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { userland } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { __TURBOPACK__default__export__ } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Merged (module eval)
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 13
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 14
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 15
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 16
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 17
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 18
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 19
};
"module evaluation";

```
# Entrypoints

```
{
    ModuleEvaluation: 0,
    Export(
        "unstable_getServerSideProps",
    ): 9,
    Export(
        "unstable_getStaticPaths",
    ): 11,
    Export(
        "reportWebVitals",
    ): 6,
    Export(
        "unstable_getServerProps",
    ): 8,
    Export(
        "routeModule",
    ): 7,
    Export(
        "getStaticProps",
    ): 5,
    Export(
        "config",
    ): 2,
    Export(
        "unstable_getStaticParams",
    ): 10,
    Export(
        "unstable_getStaticProps",
    ): 12,
    Export(
        "default",
    ): 1,
    Export(
        "getServerSideProps",
    ): 3,
    Export(
        "getStaticPaths",
    ): 4,
}
```


# Modules (prod)
## Part 0
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 13
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 14
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 15
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 16
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 17
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 18
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 19
};
"module evaluation";

```
## Part 1
```js
import { __TURBOPACK__default__export__ } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 19
};
export { __TURBOPACK__default__export__ as default };

```
## Part 2
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 19
};
export { config };
import { hoist } from './helpers';
import * as userland from 'VAR_USERLAND';
const config = hoist(userland, 'config');
export { hoist } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { userland } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { config } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 3
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 19
};
export { getServerSideProps };
import { hoist } from './helpers';
import * as userland from 'VAR_USERLAND';
const getServerSideProps = hoist(userland, 'getServerSideProps');
export { hoist } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { userland } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { getServerSideProps } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 4
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 19
};
export { getStaticPaths };
import { hoist } from './helpers';
import * as userland from 'VAR_USERLAND';
const getStaticPaths = hoist(userland, 'getStaticPaths');
export { hoist } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { userland } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { getStaticPaths } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 5
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 19
};
export { getStaticProps };
import { hoist } from './helpers';
import * as userland from 'VAR_USERLAND';
const getStaticProps = hoist(userland, 'getStaticProps');
export { hoist } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { userland } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { getStaticProps } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 6
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 19
};
export { reportWebVitals };
import { hoist } from './helpers';
import * as userland from 'VAR_USERLAND';
const reportWebVitals = hoist(userland, 'reportWebVitals');
export { hoist } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { userland } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { reportWebVitals } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 7
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 19
};
export { routeModule };
import { PagesRouteModule } from '../../server/future/route-modules/pages/module.compiled';
import { RouteKind } from '../../server/future/route-kind';
import Document from 'VAR_MODULE_DOCUMENT';
import App from 'VAR_MODULE_APP';
import * as userland from 'VAR_USERLAND';
const routeModule = new PagesRouteModule({
    definition: {
        kind: RouteKind.PAGES,
        page: 'VAR_DEFINITION_PAGE',
        pathname: 'VAR_DEFINITION_PATHNAME',
        bundlePath: '',
        filename: ''
    },
    components: {
        App,
        Document
    },
    userland
});
export { PagesRouteModule } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { RouteKind } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { Document } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { App } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { userland } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { routeModule } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 8
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 19
};
export { unstable_getServerProps };
import { hoist } from './helpers';
import * as userland from 'VAR_USERLAND';
const unstable_getServerProps = hoist(userland, 'unstable_getServerProps');
export { hoist } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { userland } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { unstable_getServerProps } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 9
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 19
};
export { unstable_getServerSideProps };
import { hoist } from './helpers';
import * as userland from 'VAR_USERLAND';
const unstable_getServerSideProps = hoist(userland, 'unstable_getServerSideProps');
export { hoist } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { userland } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { unstable_getServerSideProps } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 10
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 19
};
export { unstable_getStaticParams };
import { hoist } from './helpers';
import * as userland from 'VAR_USERLAND';
const unstable_getStaticParams = hoist(userland, 'unstable_getStaticParams');
export { hoist } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { userland } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { unstable_getStaticParams } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 11
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 19
};
export { unstable_getStaticPaths };
import { hoist } from './helpers';
import * as userland from 'VAR_USERLAND';
const unstable_getStaticPaths = hoist(userland, 'unstable_getStaticPaths');
export { hoist } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { userland } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { unstable_getStaticPaths } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 12
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 19
};
export { unstable_getStaticProps };
import { hoist } from './helpers';
import * as userland from 'VAR_USERLAND';
const unstable_getStaticProps = hoist(userland, 'unstable_getStaticProps');
export { hoist } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { userland } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { unstable_getStaticProps } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 13
```js
import '../../server/future/route-modules/pages/module.compiled';

```
## Part 14
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 13
};
import '../../server/future/route-kind';

```
## Part 15
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 13
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 14
};
import './helpers';

```
## Part 16
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 13
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 14
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 15
};
import 'VAR_MODULE_DOCUMENT';

```
## Part 17
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 13
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 14
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 15
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 16
};
import 'VAR_MODULE_APP';

```
## Part 18
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 13
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 14
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 15
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 16
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 17
};
import 'VAR_USERLAND';

```
## Part 19
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 13
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 14
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 15
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 16
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 17
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 18
};
import { hoist } from './helpers';
import * as userland from 'VAR_USERLAND';
const __TURBOPACK__default__export__ = hoist(userland, 'default');
export { hoist } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { userland } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { __TURBOPACK__default__export__ } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Merged (module eval)
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 13
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 14
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 15
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 16
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 17
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 18
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 19
};
"module evaluation";

```
