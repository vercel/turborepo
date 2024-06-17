# Items

Count: 11

## Item 5: Stmt 0, `VarDeclarator(0)`

```js
export const order = [];

```

- Declares: `order`
- Write: `order`

## Item 6: Stmt 1, `Normal`

```js
export function func() {
    order.push("d");
}

```

- Hoisted
- Declares: `func`
- Reads (eventual): `order`
- Write: `func`
- Write (eventual): `order`

## Item 7: Stmt 2, `Normal`

```js
order.push("a");

```

- Side effects
- Reads: `order`
- Write: `order`

## Item 8: Stmt 3, `VarDeclarator(0)`

```js
const x1 = externalFunction();

```

- Side effects
- Declares: `x1`
- Write: `x1`

## Item 9: Stmt 4, `VarDeclarator(0)`

```js
const x2 = externalFunction();

```

- Side effects
- Declares: `x2`
- Write: `x2`

## Item 10: Stmt 5, `VarDeclarator(0)`

```js
export const shared = {
    effect: order.push("b")
};

```

- Declares: `shared`
- Reads: `order`
- Write: `shared`, `order`

## Item 11: Stmt 6, `Normal`

```js
order.push("c");

```

- Side effects
- Reads: `order`
- Write: `order`

# Phase 1
```mermaid
graph TD
    Item1;
    Item1["ModuleEvaluation"];
    Item2;
    Item2["export func"];
    Item3;
    Item3["export order"];
    Item4;
    Item4["export shared"];
    Item5;
    Item6;
    Item7;
    Item8;
    Item9;
    Item10;
    Item11;
```
# Phase 2
```mermaid
graph TD
    Item1;
    Item1["ModuleEvaluation"];
    Item2;
    Item2["export func"];
    Item3;
    Item3["export order"];
    Item4;
    Item4["export shared"];
    Item5;
    Item6;
    Item7;
    Item8;
    Item9;
    Item10;
    Item11;
    Item2 --> Item6;
    Item5 -.-> Item3;
    Item7 --> Item5;
    Item7 -.-> Item3;
    Item8 --> Item7;
    Item8 -.-> Item3;
    Item9 --> Item7;
    Item9 --> Item8;
    Item9 -.-> Item3;
    Item10 --> Item7;
    Item10 --> Item5;
    Item10 -.-> Item4;
    Item10 -.-> Item3;
    Item11 --> Item10;
    Item11 --> Item5;
    Item11 -.-> Item3;
    Item11 --> Item7;
    Item11 --> Item8;
    Item11 --> Item9;
```
# Phase 3
```mermaid
graph TD
    Item1;
    Item1["ModuleEvaluation"];
    Item2;
    Item2["export func"];
    Item3;
    Item3["export order"];
    Item4;
    Item4["export shared"];
    Item5;
    Item6;
    Item7;
    Item8;
    Item9;
    Item10;
    Item11;
    Item2 --> Item6;
    Item5 -.-> Item3;
    Item7 --> Item5;
    Item7 -.-> Item3;
    Item8 --> Item7;
    Item8 -.-> Item3;
    Item9 --> Item7;
    Item9 --> Item8;
    Item9 -.-> Item3;
    Item10 --> Item7;
    Item10 --> Item5;
    Item10 -.-> Item4;
    Item10 -.-> Item3;
    Item11 --> Item10;
    Item11 --> Item5;
    Item11 -.-> Item3;
    Item11 --> Item7;
    Item11 --> Item8;
    Item11 --> Item9;
    Item6 --> Item11;
    Item6 -.-> Item3;
```
# Phase 4
```mermaid
graph TD
    Item1;
    Item1["ModuleEvaluation"];
    Item2;
    Item2["export func"];
    Item3;
    Item3["export order"];
    Item4;
    Item4["export shared"];
    Item5;
    Item6;
    Item7;
    Item8;
    Item9;
    Item10;
    Item11;
    Item2 --> Item6;
    Item5 -.-> Item3;
    Item7 --> Item5;
    Item7 -.-> Item3;
    Item8 --> Item7;
    Item8 -.-> Item3;
    Item9 --> Item7;
    Item9 --> Item8;
    Item9 -.-> Item3;
    Item10 --> Item7;
    Item10 --> Item5;
    Item10 -.-> Item4;
    Item10 -.-> Item3;
    Item11 --> Item10;
    Item11 --> Item5;
    Item11 -.-> Item3;
    Item11 --> Item7;
    Item11 --> Item8;
    Item11 --> Item9;
    Item6 --> Item11;
    Item6 -.-> Item3;
    Item1 --> Item7;
    Item1 --> Item8;
    Item1 --> Item9;
    Item1 --> Item11;
    Item3 --> Item11;
    Item4 --> Item10;
```
# Final
```mermaid
graph TD
    N0["Items: [ItemId(ModuleEvaluation)]"];
    N1["Items: [ItemId(Export((&quot;func&quot;, #2), &quot;func&quot;)), ItemId(1, Normal)]"];
    N2["Items: [ItemId(Export((&quot;order&quot;, #2), &quot;order&quot;))]"];
    N3["Items: [ItemId(Export((&quot;shared&quot;, #2), &quot;shared&quot;))]"];
    N4["Items: [ItemId(0, VarDeclarator(0))]"];
    N5["Items: [ItemId(2, Normal)]"];
    N6["Items: [ItemId(3, VarDeclarator(0))]"];
    N7["Items: [ItemId(4, VarDeclarator(0))]"];
    N8["Items: [ItemId(5, VarDeclarator(0))]"];
    N9["Items: [ItemId(6, Normal)]"];
    N0 --> N5;
    N0 --> N6;
    N0 --> N7;
    N0 --> N9;
    N1 --> N9;
    N1 --> N2;
    N2 --> N9;
    N3 --> N8;
    N4 --> N2;
    N5 --> N4;
    N5 --> N2;
    N6 --> N5;
    N6 --> N2;
    N7 --> N5;
    N7 --> N6;
    N7 --> N2;
    N8 --> N5;
    N8 --> N4;
    N8 --> N3;
    N8 --> N2;
    N9 --> N8;
    N9 --> N4;
    N9 --> N2;
    N9 --> N5;
    N9 --> N6;
    N9 --> N7;
```
# Entrypoints

```
{
    ModuleEvaluation: 0,
    Export(
        "order",
    ): 2,
    Export(
        "func",
    ): 1,
    Export(
        "shared",
    ): 3,
}
```


# Modules (dev)
## Part 0
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 6
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 7
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 9
};
"module evaluation";

```
## Part 1
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 9
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 2
};
export { func };
function func() {
    order.push("d");
}
export { func } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 2
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 9
};
export { order };

```
## Part 3
```js
import { shared } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 8
};
export { shared };

```
## Part 4
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 2
};
const order = [];
export { order } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 5
```js
import { order } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 4
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 2
};
order.push("a");

```
## Part 6
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 2
};
const x1 = externalFunction();
export { x1 } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 7
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 6
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 2
};
const x2 = externalFunction();
export { x2 } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 8
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
import { order } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 4
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 3
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 2
};
const shared = {
    effect: order.push("b")
};
export { shared } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 9
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 8
};
import { order } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 4
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 2
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 6
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 7
};
order.push("c");

```
## Merged (module eval)
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 6
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 7
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 9
};
"module evaluation";

```
# Entrypoints

```
{
    ModuleEvaluation: 0,
    Export(
        "order",
    ): 2,
    Export(
        "func",
    ): 1,
    Export(
        "shared",
    ): 3,
}
```


# Modules (prod)
## Part 0
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 6
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 7
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 9
};
"module evaluation";

```
## Part 1
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 9
};
export { func };
function func() {
    order.push("d");
}
export { func } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 2
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 9
};
export { order };

```
## Part 3
```js
import { shared } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 8
};
export { shared };

```
## Part 4
```js
const order = [];
export { order } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 5
```js
import { order } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 4
};
order.push("a");

```
## Part 6
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
const x1 = externalFunction();
export { x1 } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 7
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 6
};
const x2 = externalFunction();
export { x2 } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 8
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
import { order } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 4
};
const shared = {
    effect: order.push("b")
};
export { shared } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 9
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 8
};
import { order } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 4
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 6
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 7
};
order.push("c");

```
## Merged (module eval)
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 6
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 7
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 9
};
"module evaluation";

```
