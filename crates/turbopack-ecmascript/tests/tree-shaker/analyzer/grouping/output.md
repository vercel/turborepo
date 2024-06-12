# Items

Count: 13

## Item 1: Stmt 0, `VarDeclarator(0)`

```js
let x = 1;

```

- Declares: `x`
- Write: `x`

## Item 2: Stmt 1, `Normal`

```js
x = 2;

```

- Write: `x`

## Item 3: Stmt 2, `Normal`

```js
x = 3;

```

- Write: `x`

## Item 4: Stmt 3, `Normal`

```js
x = 4;

```

- Write: `x`

## Item 5: Stmt 4, `Normal`

```js
x = 5;

```

- Write: `x`

## Item 6: Stmt 5, `Normal`

```js
x += 6;

```

- Reads: `x`
- Write: `x`

## Item 7: Stmt 6, `Normal`

```js
x += 7;

```

- Reads: `x`
- Write: `x`

## Item 8: Stmt 7, `Normal`

```js
x += 8;

```

- Reads: `x`
- Write: `x`

## Item 9: Stmt 8, `Normal`

```js
x += 9;

```

- Reads: `x`
- Write: `x`

## Item 10: Stmt 10, `VarDeclarator(0)`

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
    Item2;
    Item3;
    Item4;
    Item5;
    Item6;
    Item7;
    Item8;
    Item9;
    Item10;
    Item11;
    Item11["ModuleEvaluation"];
    Item12;
    Item12["export x"];
    Item13;
    Item13["export y"];
```
# Phase 2
```mermaid
graph TD
    Item1;
    Item2;
    Item3;
    Item4;
    Item5;
    Item6;
    Item7;
    Item8;
    Item9;
    Item10;
    Item11;
    Item11["ModuleEvaluation"];
    Item12;
    Item12["export x"];
    Item13;
    Item13["export y"];
    Item1 -.-> Item1;
    Item2 -.-> Item1;
    Item3 -.-> Item1;
    Item4 -.-> Item1;
    Item5 -.-> Item1;
    Item6 --> Item1;
    Item6 --> Item5;
    Item7 --> Item1;
    Item7 --> Item5;
    Item7 --> Item6;
    Item8 --> Item1;
    Item8 --> Item5;
    Item8 --> Item6;
    Item8 --> Item7;
    Item9 --> Item1;
    Item9 --> Item5;
    Item9 --> Item6;
    Item9 --> Item7;
    Item9 --> Item8;
    Item10 --> Item1;
    Item10 --> Item5;
    Item10 --> Item6;
    Item10 --> Item7;
    Item10 --> Item8;
    Item10 --> Item9;
    Item10 -.-> Item10;
```
# Phase 3
```mermaid
graph TD
    Item1;
    Item2;
    Item3;
    Item4;
    Item5;
    Item6;
    Item7;
    Item8;
    Item9;
    Item10;
    Item11;
    Item11["ModuleEvaluation"];
    Item12;
    Item12["export x"];
    Item13;
    Item13["export y"];
    Item1 -.-> Item1;
    Item2 -.-> Item1;
    Item3 -.-> Item1;
    Item4 -.-> Item1;
    Item5 -.-> Item1;
    Item6 --> Item1;
    Item6 --> Item5;
    Item7 --> Item1;
    Item7 --> Item5;
    Item7 --> Item6;
    Item8 --> Item1;
    Item8 --> Item5;
    Item8 --> Item6;
    Item8 --> Item7;
    Item9 --> Item1;
    Item9 --> Item5;
    Item9 --> Item6;
    Item9 --> Item7;
    Item9 --> Item8;
    Item10 --> Item1;
    Item10 --> Item5;
    Item10 --> Item6;
    Item10 --> Item7;
    Item10 --> Item8;
    Item10 --> Item9;
    Item10 -.-> Item10;
```
# Phase 4
```mermaid
graph TD
    Item1;
    Item2;
    Item3;
    Item4;
    Item5;
    Item6;
    Item7;
    Item8;
    Item9;
    Item10;
    Item11;
    Item11["ModuleEvaluation"];
    Item12;
    Item12["export x"];
    Item13;
    Item13["export y"];
    Item1 -.-> Item1;
    Item2 -.-> Item1;
    Item3 -.-> Item1;
    Item4 -.-> Item1;
    Item5 -.-> Item1;
    Item6 --> Item1;
    Item6 --> Item5;
    Item7 --> Item1;
    Item7 --> Item5;
    Item7 --> Item6;
    Item8 --> Item1;
    Item8 --> Item5;
    Item8 --> Item6;
    Item8 --> Item7;
    Item9 --> Item1;
    Item9 --> Item5;
    Item9 --> Item6;
    Item9 --> Item7;
    Item9 --> Item8;
    Item10 --> Item1;
    Item10 --> Item5;
    Item10 --> Item6;
    Item10 --> Item7;
    Item10 --> Item8;
    Item10 --> Item9;
    Item10 -.-> Item10;
    Item12 --> Item1;
    Item12 --> Item5;
    Item12 --> Item6;
    Item12 --> Item7;
    Item12 --> Item8;
    Item12 --> Item9;
    Item13 --> Item10;
```
# Final
```mermaid
graph TD
    N0["Items: [ItemId(ModuleEvaluation)]"];
    N1["Items: [ItemId(Export((&quot;x&quot;, #2), &quot;x&quot;))]"];
    N2["Items: [ItemId(Export((&quot;y&quot;, #2), &quot;y&quot;)), ItemId(10, VarDeclarator(0))]"];
    N3["Items: [ItemId(0, VarDeclarator(0))]"];
    N4["Items: [ItemId(4, Normal)]"];
    N5["Items: [ItemId(5, Normal)]"];
    N6["Items: [ItemId(6, Normal)]"];
    N7["Items: [ItemId(7, Normal)]"];
    N8["Items: [ItemId(8, Normal)]"];
    N1 --> N3;
    N1 --> N4;
    N1 --> N5;
    N1 --> N6;
    N1 --> N7;
    N1 --> N8;
    N2 --> N3;
    N2 --> N4;
    N2 --> N5;
    N2 --> N6;
    N2 --> N7;
    N2 --> N8;
    N4 --> N3;
    N5 --> N3;
    N5 --> N4;
    N6 --> N3;
    N6 --> N4;
    N6 --> N5;
    N7 --> N3;
    N7 --> N4;
    N7 --> N5;
    N7 --> N6;
    N8 --> N3;
    N8 --> N4;
    N8 --> N5;
    N8 --> N6;
    N8 --> N7;
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
import { x } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 3
};
import "__TURBOPACK_PART__" assert {
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
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 8
};
export { x as x };

```
## Part 2
```js
import { x } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 3
};
import "__TURBOPACK_PART__" assert {
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
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 8
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

```
## Part 5
```js
import { x } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 3
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 4
};
x += 6;
export { x } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 6
```js
import { x } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 3
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 4
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
x += 7;
export { x } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 7
```js
import { x } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 3
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 4
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 6
};
x += 8;
export { x } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 8
```js
import { x } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 3
};
import "__TURBOPACK_PART__" assert {
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
x += 9;
export { x } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

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
import { x } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 3
};
import "__TURBOPACK_PART__" assert {
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
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 8
};
export { x as x };

```
## Part 2
```js
import { x } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 3
};
import "__TURBOPACK_PART__" assert {
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
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 8
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
x = 5;
export { x } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 5
```js
import { x } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 3
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 4
};
x += 6;
export { x } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 6
```js
import { x } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 3
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 4
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
x += 7;
export { x } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 7
```js
import { x } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 3
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 4
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 6
};
x += 8;
export { x } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 8
```js
import { x } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 3
};
import "__TURBOPACK_PART__" assert {
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
x += 9;
export { x } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Merged (module eval)
```js
"module evaluation";

```
