# Items

Count: 6

## Item 4: Stmt 0, `Normal`

```js
await Promise.resolve();

```

- Side effects

## Item 5: Stmt 1, `VarDeclarator(0)`

```js
export const effects = [];

```

- Declares: `effects`
- Write: `effects`

## Item 6: Stmt 2, `Normal`

```js
export function effect(name) {
    effects.push(name);
}

```

- Hoisted
- Declares: `effect`
- Reads (eventual): `effects`
- Write: `effect`
- Write (eventual): `effects`

# Phase 1
```mermaid
graph TD
    Item1;
    Item1["ModuleEvaluation"];
    Item2;
    Item2["export effect"];
    Item3;
    Item3["export effects"];
    Item4;
    Item5;
    Item6;
```
# Phase 2
```mermaid
graph TD
    Item1;
    Item1["ModuleEvaluation"];
    Item2;
    Item2["export effect"];
    Item3;
    Item3["export effects"];
    Item4;
    Item5;
    Item6;
    Item2 --> Item6;
    Item4 -.-> Item3;
    Item5 -.-> Item3;
```
# Phase 3
```mermaid
graph TD
    Item1;
    Item1["ModuleEvaluation"];
    Item2;
    Item2["export effect"];
    Item3;
    Item3["export effects"];
    Item4;
    Item5;
    Item6;
    Item2 --> Item6;
    Item4 -.-> Item3;
    Item5 -.-> Item3;
    Item6 --> Item5;
    Item6 -.-> Item3;
```
# Phase 4
```mermaid
graph TD
    Item1;
    Item1["ModuleEvaluation"];
    Item2;
    Item2["export effect"];
    Item3;
    Item3["export effects"];
    Item4;
    Item5;
    Item6;
    Item2 --> Item6;
    Item4 -.-> Item3;
    Item5 -.-> Item3;
    Item6 --> Item5;
    Item6 -.-> Item3;
    Item1 --> Item4;
    Item3 --> Item5;
```
# Final
```mermaid
graph TD
    N0["Items: [ItemId(ModuleEvaluation), ItemId(0, Normal)]"];
    N1["Items: [ItemId(Export((&quot;effect&quot;, #2), &quot;effect&quot;)), ItemId(2, Normal)]"];
    N2["Items: [ItemId(Export((&quot;effects&quot;, #2), &quot;effects&quot;))]"];
    N3["Items: [ItemId(1, VarDeclarator(0))]"];
    N0 --> N2;
    N1 --> N3;
    N1 --> N2;
    N2 --> N3;
    N3 --> N2;
```
# Entrypoints

```
{
    ModuleEvaluation: 0,
    Export(
        "effect",
    ): 1,
    Export(
        "effects",
    ): 2,
}
```


# Modules (dev)
## Part 0
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 2
};
"module evaluation";
await Promise.resolve();

```
## Part 1
```js
import { effects } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 3
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 2
};
export { effect };
function effect(name) {
    effects.push(name);
}
export { effect } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 2
```js
import { effects } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 3
};
export { effects };

```
## Part 3
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 2
};
const effects = [];
export { effects } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Merged (module eval)
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 2
};
"module evaluation";
await Promise.resolve();

```
# Entrypoints

```
{
    ModuleEvaluation: 0,
    Export(
        "effect",
    ): 1,
    Export(
        "effects",
    ): 2,
}
```


# Modules (prod)
## Part 0
```js
"module evaluation";
await Promise.resolve();

```
## Part 1
```js
import { effects } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 3
};
export { effect };
function effect(name) {
    effects.push(name);
}
export { effect } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 2
```js
import { effects } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 3
};
export { effects };

```
## Part 3
```js
const effects = [];
export { effects } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Merged (module eval)
```js
"module evaluation";
await Promise.resolve();

```
