# Items

Count: 15

## Item 5: Stmt 0, `Normal`

```js
function d1() {}

```

- Hoisted
- Declares: `d1`
- Write: `d1`

## Item 6: Stmt 1, `Normal`

```js
function d2() {}

```

- Hoisted
- Declares: `d2`
- Write: `d2`

## Item 7: Stmt 2, `Normal`

```js
function d3() {}

```

- Hoisted
- Declares: `d3`
- Write: `d3`

## Item 8: Stmt 3, `Normal`

```js
export function c1_1() {
    return c1_2();
}

```

- Hoisted
- Declares: `c1_1`
- Reads (eventual): `c1_2`
- Write: `c1_1`

## Item 9: Stmt 4, `Normal`

```js
function c1_2() {
    return c1_3(d1);
}

```

- Hoisted
- Declares: `c1_2`
- Reads (eventual): `c1_3`, `d1`
- Write: `c1_2`

## Item 10: Stmt 5, `Normal`

```js
export function c1_3() {
    return c1_1(d2);
}

```

- Hoisted
- Declares: `c1_3`
- Reads (eventual): `c1_1`, `d2`
- Write: `c1_3`

## Item 11: Stmt 6, `Normal`

```js
function c2_1() {
    return c2_2(d3);
}

```

- Hoisted
- Declares: `c2_1`
- Reads (eventual): `c2_2`, `d3`
- Write: `c2_1`

## Item 12: Stmt 7, `Normal`

```js
export function c2_2() {
    return c2_3();
}

```

- Hoisted
- Declares: `c2_2`
- Reads (eventual): `c2_3`
- Write: `c2_2`

## Item 13: Stmt 8, `Normal`

```js
function c2_3() {
    return c2_1();
}

```

- Hoisted
- Declares: `c2_3`
- Reads (eventual): `c2_1`
- Write: `c2_3`

## Item 14: Stmt 9, `Normal`

```js
c1_3();

```

- Side effects
- Reads: `c1_3`

## Item 15: Stmt 10, `Normal`

```js
c2_2();

```

- Side effects
- Reads: `c2_2`

# Phase 1
```mermaid
graph TD
    Item1;
    Item1["ModuleEvaluation"];
    Item2;
    Item2["export c1_1"];
    Item3;
    Item3["export c1_3"];
    Item4;
    Item4["export c2_2"];
    Item5;
    Item6;
    Item7;
    Item8;
    Item9;
    Item10;
    Item11;
    Item12;
    Item13;
    Item14;
    Item15;
```
# Phase 2
```mermaid
graph TD
    Item1;
    Item1["ModuleEvaluation"];
    Item2;
    Item2["export c1_1"];
    Item3;
    Item3["export c1_3"];
    Item4;
    Item4["export c2_2"];
    Item5;
    Item6;
    Item7;
    Item8;
    Item9;
    Item10;
    Item11;
    Item12;
    Item13;
    Item14;
    Item15;
    Item2 --> Item8;
    Item3 --> Item10;
    Item4 --> Item12;
    Item14 --> Item10;
    Item14 -.-> Item9;
    Item14 -.-> Item3;
    Item14 -.-> Item5;
    Item14 -.-> Item8;
    Item14 -.-> Item2;
    Item14 -.-> Item6;
    Item14 -.-> Item12;
    Item14 -.-> Item4;
    Item14 -.-> Item7;
    Item14 -.-> Item13;
    Item14 -.-> Item11;
    Item15 --> Item12;
    Item15 --> Item14;
    Item15 -.-> Item9;
    Item15 -.-> Item10;
    Item15 -.-> Item3;
    Item15 -.-> Item5;
    Item15 -.-> Item8;
    Item15 -.-> Item2;
    Item15 -.-> Item6;
    Item15 -.-> Item4;
    Item15 -.-> Item7;
    Item15 -.-> Item13;
    Item15 -.-> Item11;
```
# Phase 3
```mermaid
graph TD
    Item1;
    Item1["ModuleEvaluation"];
    Item2;
    Item2["export c1_1"];
    Item3;
    Item3["export c1_3"];
    Item4;
    Item4["export c2_2"];
    Item5;
    Item6;
    Item7;
    Item8;
    Item9;
    Item10;
    Item11;
    Item12;
    Item13;
    Item14;
    Item15;
    Item2 --> Item8;
    Item3 --> Item10;
    Item4 --> Item12;
    Item14 --> Item10;
    Item14 -.-> Item9;
    Item14 -.-> Item3;
    Item14 -.-> Item5;
    Item14 -.-> Item8;
    Item14 -.-> Item2;
    Item14 -.-> Item6;
    Item14 -.-> Item12;
    Item14 -.-> Item4;
    Item14 -.-> Item7;
    Item14 -.-> Item13;
    Item14 -.-> Item11;
    Item15 --> Item12;
    Item15 --> Item14;
    Item15 -.-> Item9;
    Item15 -.-> Item10;
    Item15 -.-> Item3;
    Item15 -.-> Item5;
    Item15 -.-> Item8;
    Item15 -.-> Item2;
    Item15 -.-> Item6;
    Item15 -.-> Item4;
    Item15 -.-> Item7;
    Item15 -.-> Item13;
    Item15 -.-> Item11;
    Item8 --> Item9;
    Item9 --> Item10;
    Item9 --> Item5;
    Item10 --> Item8;
    Item10 --> Item6;
    Item11 --> Item12;
    Item11 --> Item7;
    Item12 --> Item13;
    Item13 --> Item11;
```
# Phase 4
```mermaid
graph TD
    Item1;
    Item1["ModuleEvaluation"];
    Item2;
    Item2["export c1_1"];
    Item3;
    Item3["export c1_3"];
    Item4;
    Item4["export c2_2"];
    Item5;
    Item6;
    Item7;
    Item8;
    Item9;
    Item10;
    Item11;
    Item12;
    Item13;
    Item14;
    Item15;
    Item2 --> Item8;
    Item3 --> Item10;
    Item4 --> Item12;
    Item14 --> Item10;
    Item14 -.-> Item9;
    Item14 -.-> Item3;
    Item14 -.-> Item5;
    Item14 -.-> Item8;
    Item14 -.-> Item2;
    Item14 -.-> Item6;
    Item14 -.-> Item12;
    Item14 -.-> Item4;
    Item14 -.-> Item7;
    Item14 -.-> Item13;
    Item14 -.-> Item11;
    Item15 --> Item12;
    Item15 --> Item14;
    Item15 -.-> Item9;
    Item15 -.-> Item10;
    Item15 -.-> Item3;
    Item15 -.-> Item5;
    Item15 -.-> Item8;
    Item15 -.-> Item2;
    Item15 -.-> Item6;
    Item15 -.-> Item4;
    Item15 -.-> Item7;
    Item15 -.-> Item13;
    Item15 -.-> Item11;
    Item8 --> Item9;
    Item9 --> Item10;
    Item9 --> Item5;
    Item10 --> Item8;
    Item10 --> Item6;
    Item11 --> Item12;
    Item11 --> Item7;
    Item12 --> Item13;
    Item13 --> Item11;
    Item1 --> Item14;
    Item1 --> Item15;
```
# Final
```mermaid
graph TD
    N0["Items: [ItemId(ModuleEvaluation), ItemId(9, Normal), ItemId(10, Normal)]"];
    N1["Items: [ItemId(Export((&quot;c1_1&quot;, #2), &quot;c1_1&quot;))]"];
    N2["Items: [ItemId(Export((&quot;c1_3&quot;, #2), &quot;c1_3&quot;))]"];
    N3["Items: [ItemId(Export((&quot;c2_2&quot;, #2), &quot;c2_2&quot;))]"];
    N4["Items: [ItemId(3, Normal), ItemId(4, Normal), ItemId(5, Normal)]"];
    N5["Items: [ItemId(6, Normal), ItemId(7, Normal), ItemId(8, Normal)]"];
    N6["Items: [ItemId(0, Normal)]"];
    N7["Items: [ItemId(1, Normal)]"];
    N8["Items: [ItemId(2, Normal)]"];
    N0 --> N4;
    N0 --> N2;
    N0 --> N6;
    N0 --> N1;
    N0 --> N7;
    N0 --> N5;
    N0 --> N3;
    N0 --> N8;
    N1 --> N4;
    N2 --> N4;
    N3 --> N5;
    N4 --> N6;
    N4 --> N7;
    N5 --> N8;
```
# Entrypoints

```
{
    ModuleEvaluation: 0,
    Export(
        "c1_1",
    ): 1,
    Export(
        "c1_3",
    ): 2,
    Export(
        "c2_2",
    ): 3,
}
```


# Modules (dev)
## Part 0
```js
import { c1_3 } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 4
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 2
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 6
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 1
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 7
};
import { c2_2 } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 3
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 8
};
"module evaluation";
c1_3();
c2_2();

```
## Part 1
```js
import { c1_1 } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 4
};
export { c1_1 };

```
## Part 2
```js
import { c1_3 } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 4
};
export { c1_3 };

```
## Part 3
```js
import { c2_2 } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
export { c2_2 };

```
## Part 4
```js
import { d1 } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 6
};
import { d2 } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 7
};
function c1_1() {
    return c1_2();
}
function c1_2() {
    return c1_3(d1);
}
function c1_3() {
    return c1_1(d2);
}
export { c1_1 } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { c1_2 } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { c1_3 } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 5
```js
import { d3 } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 8
};
function c2_1() {
    return c2_2(d3);
}
function c2_2() {
    return c2_3();
}
function c2_3() {
    return c2_1();
}
export { c2_1 } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { c2_2 } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { c2_3 } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 6
```js
function d1() {}
export { d1 } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 7
```js
function d2() {}
export { d2 } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 8
```js
function d3() {}
export { d3 } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Merged (module eval)
```js
import { c1_3 } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 4
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 2
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 6
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 1
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 7
};
import { c2_2 } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 3
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 8
};
"module evaluation";
c1_3();
c2_2();

```
# Entrypoints

```
{
    ModuleEvaluation: 0,
    Export(
        "c1_1",
    ): 1,
    Export(
        "c1_3",
    ): 2,
    Export(
        "c2_2",
    ): 3,
}
```


# Modules (prod)
## Part 0
```js
import { c1_3 } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 4
};
import { c2_2 } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
"module evaluation";
c1_3();
c2_2();

```
## Part 1
```js
import { c1_1 } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 4
};
export { c1_1 };

```
## Part 2
```js
import { c1_3 } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 4
};
export { c1_3 };

```
## Part 3
```js
import { c2_2 } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
export { c2_2 };

```
## Part 4
```js
function c1_1() {
    return c1_2();
}
function c1_2() {
    return c1_3(d1);
}
function c1_3() {
    return c1_1(d2);
}
export { c1_1 } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { c1_2 } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { c1_3 } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 5
```js
function c2_1() {
    return c2_2(d3);
}
function c2_2() {
    return c2_3();
}
function c2_3() {
    return c2_1();
}
export { c2_1 } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { c2_2 } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { c2_3 } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Merged (module eval)
```js
import { c1_3 } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 4
};
import { c2_2 } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
"module evaluation";
c1_3();
c2_2();

```
# Entrypoints

```
{
    ModuleEvaluation: 0,
    Export(
        "c1_1",
    ): 1,
    Export(
        "c1_3",
    ): 2,
    Export(
        "c2_2",
    ): 3,
}
```


## Merged (c1_3)
```js
import { c1_3 } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 4
};
export { c1_3 };

```
# Entrypoints

```
{
    ModuleEvaluation: 0,
    Export(
        "c1_1",
    ): 1,
    Export(
        "c1_3",
    ): 2,
    Export(
        "c2_2",
    ): 3,
}
```


## Merged (c1_3,c2_2)
```js
import { c1_3 } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 4
};
import { c2_2 } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
export { c1_3 };
export { c2_2 };

```
