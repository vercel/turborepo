# Items

Count: 13

## Item 4: Stmt 0, `VarDeclarator(0)`

```js
let x = 1;

```

- Declares: `x`
- Write: `x`

## Item 5: Stmt 1, `Normal`

```js
x = 2;

```

- Write: `x`

## Item 6: Stmt 2, `Normal`

```js
x = 3;

```

- Write: `x`

## Item 7: Stmt 3, `Normal`

```js
x = 4;

```

- Write: `x`

## Item 8: Stmt 4, `Normal`

```js
x = 5;

```

- Write: `x`

## Item 9: Stmt 5, `Normal`

```js
x += 6;

```

- Reads: `x`
- Write: `x`

## Item 10: Stmt 6, `Normal`

```js
x += 7;

```

- Reads: `x`
- Write: `x`

## Item 11: Stmt 7, `Normal`

```js
x += 8;

```

- Reads: `x`
- Write: `x`

## Item 12: Stmt 8, `Normal`

```js
x += 9;

```

- Reads: `x`
- Write: `x`

## Item 13: Stmt 10, `VarDeclarator(0)`

```js
export const y = x;

```

- Declares: `y`
- Reads: `x`
- Write: `y`

# Phase 1
```mermaid
graph TD
    Item1;
    Item1["ModuleEvaluation"];
    Item2;
    Item2["export x"];
    Item3;
    Item3["export y"];
    Item4;
    Item5;
    Item6;
    Item7;
    Item8;
    Item9;
    Item10;
    Item11;
    Item12;
    Item13;
```
# Phase 2
```mermaid
graph TD
    Item1;
    Item1["ModuleEvaluation"];
    Item2;
    Item2["export x"];
    Item3;
    Item3["export y"];
    Item4;
    Item5;
    Item6;
    Item7;
    Item8;
    Item9;
    Item10;
    Item11;
    Item12;
    Item13;
    Item4 -.-> Item2;
    Item5 -.-> Item2;
    Item5 -.-> Item4;
    Item6 -.-> Item2;
    Item6 -.-> Item4;
    Item7 -.-> Item2;
    Item7 -.-> Item4;
    Item8 -.-> Item2;
    Item8 -.-> Item4;
    Item9 --> Item4;
    Item9 --> Item8;
    Item9 -.-> Item2;
    Item10 --> Item9;
    Item10 --> Item4;
    Item10 -.-> Item2;
    Item11 --> Item10;
    Item11 --> Item4;
    Item11 -.-> Item2;
    Item12 --> Item11;
    Item12 --> Item4;
    Item12 -.-> Item2;
    Item13 --> Item12;
    Item13 --> Item4;
    Item13 -.-> Item3;
```
# Phase 3
```mermaid
graph TD
    Item1;
    Item1["ModuleEvaluation"];
    Item2;
    Item2["export x"];
    Item3;
    Item3["export y"];
    Item4;
    Item5;
    Item6;
    Item7;
    Item8;
    Item9;
    Item10;
    Item11;
    Item12;
    Item13;
    Item4 -.-> Item2;
    Item5 -.-> Item2;
    Item5 -.-> Item4;
    Item6 -.-> Item2;
    Item6 -.-> Item4;
    Item7 -.-> Item2;
    Item7 -.-> Item4;
    Item8 -.-> Item2;
    Item8 -.-> Item4;
    Item9 --> Item4;
    Item9 --> Item8;
    Item9 -.-> Item2;
    Item10 --> Item9;
    Item10 --> Item4;
    Item10 -.-> Item2;
    Item11 --> Item10;
    Item11 --> Item4;
    Item11 -.-> Item2;
    Item12 --> Item11;
    Item12 --> Item4;
    Item12 -.-> Item2;
    Item13 --> Item12;
    Item13 --> Item4;
    Item13 -.-> Item3;
```
# Phase 4
```mermaid
graph TD
    Item1;
    Item1["ModuleEvaluation"];
    Item2;
    Item2["export x"];
    Item3;
    Item3["export y"];
    Item4;
    Item5;
    Item6;
    Item7;
    Item8;
    Item9;
    Item10;
    Item11;
    Item12;
    Item13;
    Item4 -.-> Item2;
    Item5 -.-> Item2;
    Item5 -.-> Item4;
    Item6 -.-> Item2;
    Item6 -.-> Item4;
    Item7 -.-> Item2;
    Item7 -.-> Item4;
    Item8 -.-> Item2;
    Item8 -.-> Item4;
    Item9 --> Item4;
    Item9 --> Item8;
    Item9 -.-> Item2;
    Item10 --> Item9;
    Item10 --> Item4;
    Item10 -.-> Item2;
    Item11 --> Item10;
    Item11 --> Item4;
    Item11 -.-> Item2;
    Item12 --> Item11;
    Item12 --> Item4;
    Item12 -.-> Item2;
    Item13 --> Item12;
    Item13 --> Item4;
    Item13 -.-> Item3;
    Item2 --> Item12;
    Item3 --> Item13;
```
# Final
```mermaid
graph TD
    N0["Items: [ItemId(ModuleEvaluation)]"];
    N1["Items: [ItemId(Export((&quot;x&quot;, #2), &quot;x&quot;))]"];
    N2["Items: [ItemId(Export((&quot;y&quot;, #2), &quot;y&quot;)), ItemId(10, VarDeclarator(0))]"];
    N3["Items: [ItemId(0, VarDeclarator(0))]"];
    N4["Items: [ItemId(4, Normal), ItemId(5, Normal), ItemId(6, Normal), ItemId(7, Normal), ItemId(8, Normal)]"];
    N1 --> N4;
    N2 --> N4;
    N2 --> N3;
    N3 --> N1;
    N4 --> N1;
    N4 --> N3;
```
# Entrypoints

```
{
    ModuleEvaluation: 0,
    Export(
        "y",
    ): 2,
    Export(
        "x",
    ): 1,
}
```


# Modules (dev)
## Part 0
```js
"module evaluation";

```
## Part 1
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 4
};
export { x as x };

```
## Part 2
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 4
};
import { x } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 3
};
export { y };
const y = x;
export { y } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 3
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 1
};
let x = 1;
export { x } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 4
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 1
};
import { x } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 3
};
x = 5;
x += 6;
x += 7;
x += 8;
x += 9;

```
## Merged (module eval)
```js
"module evaluation";

```
# Entrypoints

```
{
    ModuleEvaluation: 0,
    Export(
        "y",
    ): 2,
    Export(
        "x",
    ): 1,
}
```


# Modules (prod)
## Part 0
```js
"module evaluation";

```
## Part 1
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 4
};
export { x as x };

```
## Part 2
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 4
};
import { x } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 3
};
export { y };
const y = x;
export { y } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 3
```js
let x = 1;
export { x } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 4
```js
import { x } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 3
};
x = 5;
x += 6;
x += 7;
x += 8;
x += 9;

```
## Merged (module eval)
```js
"module evaluation";

```
