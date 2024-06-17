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
order.push("a");

```

- Side effects
- Reads: `order`
- Write: `order`

## Item 7: Stmt 2, `VarDeclarator(0)`

```js
const random = Math.random();

```

- Side effects
- Declares: `random`
- Write: `random`

## Item 8: Stmt 3, `VarDeclarator(0)`

```js
const shared = {
    random,
    effect: order.push("b")
};

```

- Declares: `shared`
- Reads: `random`, `order`
- Write: `shared`, `order`

## Item 9: Stmt 4, `Normal`

```js
order.push("c");

```

- Side effects
- Reads: `order`
- Write: `order`

## Item 10: Stmt 5, `VarDeclarator(0)`

```js
export const a = {
    shared,
    a: "aaaaaaaaaaa"
};

```

- Declares: `a`
- Reads: `shared`
- Write: `a`

## Item 11: Stmt 6, `VarDeclarator(0)`

```js
export const b = {
    shared,
    b: "bbbbbbbbbbb"
};

```

- Declares: `b`
- Reads: `shared`
- Write: `b`

# Phase 1
```mermaid
graph TD
    Item1;
    Item1["ModuleEvaluation"];
    Item2;
    Item2["export a"];
    Item3;
    Item3["export b"];
    Item4;
    Item4["export order"];
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
    Item2["export a"];
    Item3;
    Item3["export b"];
    Item4;
    Item4["export order"];
    Item5;
    Item6;
    Item7;
    Item8;
    Item9;
    Item10;
    Item11;
    Item5 -.-> Item4;
    Item6 --> Item5;
    Item6 -.-> Item4;
    Item7 --> Item6;
    Item8 --> Item7;
    Item8 --> Item6;
    Item8 --> Item5;
    Item8 -.-> Item4;
    Item9 --> Item8;
    Item9 --> Item5;
    Item9 -.-> Item4;
    Item9 --> Item6;
    Item9 --> Item7;
    Item10 --> Item8;
    Item10 -.-> Item2;
    Item11 --> Item8;
    Item11 -.-> Item3;
```
# Phase 3
```mermaid
graph TD
    Item1;
    Item1["ModuleEvaluation"];
    Item2;
    Item2["export a"];
    Item3;
    Item3["export b"];
    Item4;
    Item4["export order"];
    Item5;
    Item6;
    Item7;
    Item8;
    Item9;
    Item10;
    Item11;
    Item5 -.-> Item4;
    Item6 --> Item5;
    Item6 -.-> Item4;
    Item7 --> Item6;
    Item8 --> Item7;
    Item8 --> Item6;
    Item8 --> Item5;
    Item8 -.-> Item4;
    Item9 --> Item8;
    Item9 --> Item5;
    Item9 -.-> Item4;
    Item9 --> Item6;
    Item9 --> Item7;
    Item10 --> Item8;
    Item10 -.-> Item2;
    Item11 --> Item8;
    Item11 -.-> Item3;
```
# Phase 4
```mermaid
graph TD
    Item1;
    Item1["ModuleEvaluation"];
    Item2;
    Item2["export a"];
    Item3;
    Item3["export b"];
    Item4;
    Item4["export order"];
    Item5;
    Item6;
    Item7;
    Item8;
    Item9;
    Item10;
    Item11;
    Item5 -.-> Item4;
    Item6 --> Item5;
    Item6 -.-> Item4;
    Item7 --> Item6;
    Item8 --> Item7;
    Item8 --> Item6;
    Item8 --> Item5;
    Item8 -.-> Item4;
    Item9 --> Item8;
    Item9 --> Item5;
    Item9 -.-> Item4;
    Item9 --> Item6;
    Item9 --> Item7;
    Item10 --> Item8;
    Item10 -.-> Item2;
    Item11 --> Item8;
    Item11 -.-> Item3;
    Item1 --> Item6;
    Item1 --> Item7;
    Item1 --> Item9;
    Item2 --> Item10;
    Item3 --> Item11;
    Item4 --> Item9;
```
# Final
```mermaid
graph TD
    N0["Items: [ItemId(ModuleEvaluation)]"];
    N1["Items: [ItemId(Export((&quot;a&quot;, #2), &quot;a&quot;)), ItemId(5, VarDeclarator(0))]"];
    N2["Items: [ItemId(Export((&quot;b&quot;, #2), &quot;b&quot;)), ItemId(6, VarDeclarator(0))]"];
    N3["Items: [ItemId(Export((&quot;order&quot;, #2), &quot;order&quot;))]"];
    N4["Items: [ItemId(0, VarDeclarator(0))]"];
    N5["Items: [ItemId(1, Normal)]"];
    N6["Items: [ItemId(2, VarDeclarator(0))]"];
    N7["Items: [ItemId(3, VarDeclarator(0))]"];
    N8["Items: [ItemId(4, Normal)]"];
    N0 --> N5;
    N0 --> N6;
    N0 --> N8;
    N1 --> N7;
    N2 --> N7;
    N3 --> N8;
    N4 --> N3;
    N5 --> N4;
    N5 --> N3;
    N6 --> N5;
    N7 --> N6;
    N7 --> N5;
    N7 --> N4;
    N7 --> N3;
    N8 --> N7;
    N8 --> N4;
    N8 --> N3;
    N8 --> N5;
    N8 --> N6;
```
# Entrypoints

```
{
    ModuleEvaluation: 0,
    Export(
        "order",
    ): 3,
    Export(
        "b",
    ): 2,
    Export(
        "a",
    ): 1,
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
    __turbopack_part__: 8
};
"module evaluation";

```
## Part 1
```js
import { shared } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 7
};
export { a };
const a = {
    shared,
    a: "aaaaaaaaaaa"
};
export { a } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 2
```js
import { shared } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 7
};
export { b };
const b = {
    shared,
    b: "bbbbbbbbbbb"
};
export { b } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 3
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 8
};
export { order };

```
## Part 4
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 3
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
    __turbopack_part__: 3
};
order.push("a");

```
## Part 6
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
const random = Math.random();
export { random } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 7
```js
import { random } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 6
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
import { order } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 4
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 3
};
const shared = {
    random,
    effect: order.push("b")
};
export { shared } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 8
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 7
};
import { order } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 4
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 3
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 6
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
    __turbopack_part__: 8
};
"module evaluation";

```
# Entrypoints

```
{
    ModuleEvaluation: 0,
    Export(
        "order",
    ): 3,
    Export(
        "b",
    ): 2,
    Export(
        "a",
    ): 1,
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
    __turbopack_part__: 8
};
"module evaluation";

```
## Part 1
```js
import { shared } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 7
};
export { a };
const a = {
    shared,
    a: "aaaaaaaaaaa"
};
export { a } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 2
```js
import { shared } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 7
};
export { b };
const b = {
    shared,
    b: "bbbbbbbbbbb"
};
export { b } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 3
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 8
};
export { order };

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
const random = Math.random();
export { random } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 7
```js
import { random } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 6
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
import { order } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 4
};
const shared = {
    random,
    effect: order.push("b")
};
export { shared } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 8
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 7
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
    __turbopack_part__: 8
};
"module evaluation";

```
