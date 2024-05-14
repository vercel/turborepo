# Items

Count: 19

## Item 1: Stmt 0, `Normal`

```js
export function external1() {
    return internal() + foobar;
}

```

- Hoisted
- Declares: `external1`
- Reads (eventual): `internal`, `foobar`
- Write: `external1`

## Item 2: Stmt 1, `ImportOfModule`

```js
import { upper } from "module";

```

- Hoisted
- Side effects

## Item 3: Stmt 1, `ImportBinding(0)`

```js
import { upper } from "module";

```

- Hoisted
- Declares: `upper`

## Item 4: Stmt 2, `VarDeclarator(0)`

```js
export let foobar = "foo";

```

- Declares: `foobar`
- Write: `foobar`

## Item 5: Stmt 3, `VarDeclarator(0)`

```js
export const foo = foobar;

```

- Declares: `foo`
- Reads: `foobar`
- Write: `foo`

## Item 6: Stmt 4, `VarDeclarator(0)`

```js
const bar = "bar";

```

- Declares: `bar`
- Write: `bar`

## Item 7: Stmt 5, `Normal`

```js
foobar += bar;

```

- Side effects
- Reads: `bar`
- Write: `foobar`

## Item 8: Stmt 6, `VarDeclarator(0)`

```js
let foobarCopy = foobar;

```

- Declares: `foobarCopy`
- Reads: `foobar`
- Write: `foobarCopy`

## Item 9: Stmt 7, `Normal`

```js
foobar += "foo";

```

- Side effects
- Write: `foobar`

## Item 10: Stmt 8, `Normal`

```js
console.log(foobarCopy);

```

- Side effects
- Reads: `console`, `foobarCopy`

## Item 11: Stmt 9, `Normal`

```js
foobarCopy += "Unused";

```

- Side effects
- Write: `foobarCopy`

## Item 12: Stmt 10, `Normal`

```js
function internal() {
    return upper(foobar);
}

```

- Hoisted
- Declares: `internal`
- Reads (eventual): `upper`, `foobar`
- Write: `internal`

## Item 13: Stmt 11, `Normal`

```js
export function external2() {
    foobar += ".";
}

```

- Hoisted
- Declares: `external2`
- Write: `external2`
- Write (eventual): `foobar`

## Item 14: Stmt 12, `ImportOfModule`

```js
import "other";

```

- Hoisted
- Side effects

# Phase 1
```mermaid
graph TD
    Item3;
    Item1;
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
    Item14;
    Item2;
    Item15;
    Item15["ModuleEvaluation"];
    Item16;
    Item16["export external1"];
    Item17;
    Item17["export foobar"];
    Item18;
    Item18["export foo"];
    Item19;
    Item19["export external2"];
    Item2 --> Item1;
```
# Phase 2
```mermaid
graph TD
    Item3;
    Item1;
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
    Item14;
    Item2;
    Item15;
    Item15["ModuleEvaluation"];
    Item16;
    Item16["export external1"];
    Item17;
    Item17["export foobar"];
    Item18;
    Item18["export foo"];
    Item19;
    Item19["export external2"];
    Item2 --> Item1;
    Item6 --> Item5;
    Item8 --> Item7;
    Item8 -.-> Item6;
    Item8 --> Item1;
    Item8 --> Item2;
    Item8 -.-> Item13;
    Item8 -.-> Item5;
    Item8 -.-> Item4;
    Item9 --> Item5;
    Item9 --> Item8;
    Item10 -.-> Item6;
    Item10 -.-> Item9;
    Item10 --> Item1;
    Item10 --> Item2;
    Item10 --> Item8;
    Item10 -.-> Item13;
    Item10 -.-> Item5;
    Item10 -.-> Item4;
    Item11 --> Item9;
    Item11 --> Item1;
    Item11 --> Item2;
    Item11 --> Item8;
    Item11 --> Item10;
    Item11 -.-> Item13;
    Item11 -.-> Item5;
    Item11 -.-> Item6;
    Item11 -.-> Item4;
    Item12 --> Item11;
    Item12 --> Item1;
    Item12 --> Item2;
    Item12 --> Item8;
    Item12 --> Item10;
    Item12 -.-> Item13;
    Item12 -.-> Item5;
    Item12 -.-> Item6;
    Item12 -.-> Item9;
    Item12 -.-> Item4;
```
# Phase 3
```mermaid
graph TD
    Item3;
    Item1;
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
    Item14;
    Item2;
    Item15;
    Item15["ModuleEvaluation"];
    Item16;
    Item16["export external1"];
    Item17;
    Item17["export foobar"];
    Item18;
    Item18["export foo"];
    Item19;
    Item19["export external2"];
    Item2 --> Item1;
    Item6 --> Item5;
    Item8 --> Item7;
    Item8 -.-> Item6;
    Item8 --> Item1;
    Item8 --> Item2;
    Item8 -.-> Item13;
    Item8 -.-> Item5;
    Item8 -.-> Item4;
    Item9 --> Item5;
    Item9 --> Item8;
    Item10 -.-> Item6;
    Item10 -.-> Item9;
    Item10 --> Item1;
    Item10 --> Item2;
    Item10 --> Item8;
    Item10 -.-> Item13;
    Item10 -.-> Item5;
    Item10 -.-> Item4;
    Item11 --> Item9;
    Item11 --> Item1;
    Item11 --> Item2;
    Item11 --> Item8;
    Item11 --> Item10;
    Item11 -.-> Item13;
    Item11 -.-> Item5;
    Item11 -.-> Item6;
    Item11 -.-> Item4;
    Item12 --> Item11;
    Item12 --> Item1;
    Item12 --> Item2;
    Item12 --> Item8;
    Item12 --> Item10;
    Item12 -.-> Item13;
    Item12 -.-> Item5;
    Item12 -.-> Item6;
    Item12 -.-> Item9;
    Item12 -.-> Item4;
    Item3 --> Item13;
    Item3 --> Item5;
    Item3 --> Item10;
    Item13 --> Item4;
    Item13 --> Item5;
    Item13 --> Item10;
    Item14 -.-> Item6;
    Item14 -.-> Item9;
```
# Phase 4
```mermaid
graph TD
    Item3;
    Item1;
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
    Item14;
    Item2;
    Item15;
    Item15["ModuleEvaluation"];
    Item16;
    Item16["export external1"];
    Item17;
    Item17["export foobar"];
    Item18;
    Item18["export foo"];
    Item19;
    Item19["export external2"];
    Item2 --> Item1;
    Item6 --> Item5;
    Item8 --> Item7;
    Item8 -.-> Item6;
    Item8 --> Item1;
    Item8 --> Item2;
    Item8 -.-> Item13;
    Item8 -.-> Item5;
    Item8 -.-> Item4;
    Item9 --> Item5;
    Item9 --> Item8;
    Item10 -.-> Item6;
    Item10 -.-> Item9;
    Item10 --> Item1;
    Item10 --> Item2;
    Item10 --> Item8;
    Item10 -.-> Item13;
    Item10 -.-> Item5;
    Item10 -.-> Item4;
    Item11 --> Item9;
    Item11 --> Item1;
    Item11 --> Item2;
    Item11 --> Item8;
    Item11 --> Item10;
    Item11 -.-> Item13;
    Item11 -.-> Item5;
    Item11 -.-> Item6;
    Item11 -.-> Item4;
    Item12 --> Item11;
    Item12 --> Item1;
    Item12 --> Item2;
    Item12 --> Item8;
    Item12 --> Item10;
    Item12 -.-> Item13;
    Item12 -.-> Item5;
    Item12 -.-> Item6;
    Item12 -.-> Item9;
    Item12 -.-> Item4;
    Item3 --> Item13;
    Item3 --> Item5;
    Item3 --> Item10;
    Item13 --> Item4;
    Item13 --> Item5;
    Item13 --> Item10;
    Item14 -.-> Item6;
    Item14 -.-> Item9;
    Item15 --> Item1;
    Item15 --> Item2;
    Item15 --> Item8;
    Item15 --> Item10;
    Item15 --> Item11;
    Item15 --> Item12;
    Item16 --> Item3;
    Item17 --> Item5;
    Item17 --> Item10;
    Item18 --> Item6;
    Item19 --> Item14;
```
# Final
```mermaid
graph TD
    N0["Items: [ItemId(ModuleEvaluation), ItemId(1, ImportBinding(0)), ItemId(8, Normal), ItemId(9, Normal)]"];
    N1["Items: [ItemId(Export((&quot;external1&quot;, #2), &quot;external1&quot;)), ItemId(0, Normal)]"];
    N2["Items: [ItemId(Export((&quot;foobar&quot;, #2), &quot;foobar&quot;))]"];
    N3["Items: [ItemId(Export((&quot;foo&quot;, #2), &quot;foo&quot;))]"];
    N4["Items: [ItemId(Export((&quot;external2&quot;, #2), &quot;external2&quot;)), ItemId(11, Normal)]"];
    N5["Items: [ItemId(1, ImportBinding(0)), ItemId(5, Normal), ItemId(6, VarDeclarator(0)), ItemId(7, Normal), ItemId(10, Normal)]"];
    N6["Items: [ItemId(1, ImportOfModule)]"];
    N7["Items: [ItemId(12, ImportOfModule)]"];
    N8["Items: [ItemId(2, VarDeclarator(0))]"];
    N9["Items: [ItemId(3, VarDeclarator(0))]"];
    N10["Items: [ItemId(4, VarDeclarator(0))]"];
    N0 --> N6;
    N0 --> N7;
    N0 --> N5;
    N0 --> N8;
    N0 --> N9;
    N1 --> N5;
    N1 --> N8;
    N2 --> N8;
    N2 --> N5;
    N3 --> N9;
    N4 --> N9;
    N4 --> N5;
    N5 --> N10;
    N5 --> N9;
    N5 --> N6;
    N5 --> N7;
    N5 --> N8;
    N7 --> N6;
    N9 --> N8;
```
# Entrypoints

```
{
    ModuleEvaluation: 0,
    Export(
        "external1",
    ): 1,
    Export(
        "foo",
    ): 3,
    Export(
        "foobar",
    ): 2,
    Export(
        "external2",
    ): 4,
}
```


# Modules (dev)
## Part 0
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 6
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 7
};
import { foobarCopy } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 8
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 9
};
"module evaluation";
import { upper } from "module";
console.log(foobarCopy);
foobarCopy += "Unused";

```
## Part 1
```js
import { foobar, internal } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 8
};
export { external1 };
function external1() {
    return internal() + foobar;
}
export { external1 } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 2
```js
import { foobar } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 8
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
export { foobar };

```
## Part 3
```js
import { foo } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 9
};
export { foo };

```
## Part 4
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 9
};
import { foobar } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
export { external2 };
function external2() {
    foobar += ".";
}
export { external2 } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 5
```js
import { bar } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 10
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 9
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 6
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 7
};
import { foobar } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 8
};
import { upper } from "module";
foobar += bar;
let foobarCopy = foobar;
foobar += "foo";
function internal() {
    return upper(foobar);
}
export { foobarCopy } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { internal } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 6
```js
import "module";

```
## Part 7
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 6
};
import "other";

```
## Part 8
```js
let foobar = "foo";
export { foobar } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 9
```js
import { foobar } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 8
};
const foo = foobar;
export { foo } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 10
```js
const bar = "bar";
export { bar } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Merged (module eval)
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 6
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 7
};
import { foobarCopy } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 8
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 9
};
import { upper } from "module";
"module evaluation";
console.log(foobarCopy);
foobarCopy += "Unused";

```
# Entrypoints

```
{
    ModuleEvaluation: 0,
    Export(
        "external1",
    ): 1,
    Export(
        "foo",
    ): 3,
    Export(
        "foobar",
    ): 2,
    Export(
        "external2",
    ): 4,
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
import { foobar } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 9
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 10
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 7
};
"module evaluation";
let foobarCopy = foobar;
console.log(foobarCopy);
foobarCopy += "Unused";
export { foobarCopy } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 1
```js
import { foobar } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 7
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 10
};
export { external1 };
function external1() {
    return internal() + foobar;
}
import { upper } from "module";
function internal() {
    return upper(foobar);
}
export { external1 } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { internal } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 2
```js
import { foobar } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 7
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 10
};
export { foobar };

```
## Part 3
```js
import { foobar } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 7
};
export { foo };
const foo = foobar;
export { foo } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 4
```js
export { external2 };
function external2() {
    foobar += ".";
}
export { external2 } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 5
```js
import "module";

```
## Part 6
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
import "other";

```
## Part 7
```js
let foobar = "foo";
export { foobar } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 8
```js
const bar = "bar";
export { bar } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 9
```js
import { bar } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 8
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 6
};
foobar += bar;
export { foobar } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 10
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 6
};
import { foobar } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 9
};
foobar += "foo";

```
## Merged (module eval)
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 6
};
import { foobar } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 9
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 10
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 7
};
"module evaluation";
let foobarCopy = foobar;
console.log(foobarCopy);
foobarCopy += "Unused";
export { foobarCopy } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
