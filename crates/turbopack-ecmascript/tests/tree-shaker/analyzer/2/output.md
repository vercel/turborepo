# Items

Count: 19

## Item 6: Stmt 0, `Normal`

```js
export function external1() {
    return internal() + foobar;
}

```

- Hoisted
- Declares: `external1`
- Reads (eventual): `internal`, `foobar`
- Write: `external1`

## Item 7: Stmt 1, `ImportOfModule`

```js
import { upper } from "module";

```

- Hoisted
- Side effects

## Item 8: Stmt 1, `ImportBinding(0)`

```js
import { upper } from "module";

```

- Hoisted
- Declares: `upper`

## Item 9: Stmt 2, `VarDeclarator(0)`

```js
export let foobar = "foo";

```

- Declares: `foobar`
- Write: `foobar`

## Item 10: Stmt 3, `VarDeclarator(0)`

```js
export const foo = foobar;

```

- Declares: `foo`
- Reads: `foobar`
- Write: `foo`

## Item 11: Stmt 4, `VarDeclarator(0)`

```js
const bar = "bar";

```

- Declares: `bar`
- Write: `bar`

## Item 12: Stmt 5, `Normal`

```js
foobar += bar;

```

- Reads: `bar`, `foobar`
- Write: `foobar`

## Item 13: Stmt 6, `VarDeclarator(0)`

```js
let foobarCopy = foobar;

```

- Declares: `foobarCopy`
- Reads: `foobar`
- Write: `foobarCopy`

## Item 14: Stmt 7, `Normal`

```js
foobar += "foo";

```

- Reads: `foobar`
- Write: `foobar`

## Item 15: Stmt 8, `Normal`

```js
console.log(foobarCopy);

```

- Side effects
- Reads: `foobarCopy`

## Item 16: Stmt 9, `Normal`

```js
foobarCopy += "Unused";

```

- Reads: `foobarCopy`
- Write: `foobarCopy`

## Item 17: Stmt 10, `Normal`

```js
function internal() {
    return upper(foobar);
}

```

- Hoisted
- Declares: `internal`
- Reads (eventual): `upper`, `foobar`
- Write: `internal`

## Item 18: Stmt 11, `Normal`

```js
export function external2() {
    foobar += ".";
}

```

- Hoisted
- Declares: `external2`
- Write: `external2`
- Write (eventual): `foobar`

## Item 19: Stmt 12, `ImportOfModule`

```js
import "other";

```

- Hoisted
- Side effects

# Phase 1
```mermaid
graph TD
    Item3;
    Item3["ModuleEvaluation"];
    Item4;
    Item4["export external1"];
    Item5;
    Item5["export external2"];
    Item6;
    Item6["export foo"];
    Item7;
    Item7["export foobar"];
    Item8;
    Item1;
    Item9;
    Item10;
    Item11;
    Item12;
    Item13;
    Item14;
    Item15;
    Item16;
    Item17;
    Item18;
    Item19;
    Item2;
    Item2 --> Item1;
```
# Phase 2
```mermaid
graph TD
    Item3;
    Item3["ModuleEvaluation"];
    Item4;
    Item4["export external1"];
    Item5;
    Item5["export external2"];
    Item6;
    Item6["export foo"];
    Item7;
    Item7["export foobar"];
    Item8;
    Item1;
    Item9;
    Item10;
    Item11;
    Item12;
    Item13;
    Item14;
    Item15;
    Item16;
    Item17;
    Item18;
    Item19;
    Item2;
    Item2 --> Item1;
    Item4 --> Item8;
    Item5 --> Item19;
    Item10 -.-> Item7;
    Item11 --> Item10;
    Item11 -.-> Item6;
    Item13 --> Item12;
    Item13 --> Item10;
    Item13 -.-> Item7;
    Item13 -.-> Item11;
    Item14 --> Item13;
    Item14 --> Item10;
    Item15 --> Item13;
    Item15 --> Item10;
    Item15 -.-> Item7;
    Item15 -.-> Item11;
    Item15 -.-> Item14;
    Item16 --> Item14;
    Item16 --> Item1;
    Item16 --> Item2;
    Item16 -.-> Item18;
    Item16 -.-> Item15;
    Item16 -.-> Item7;
    Item16 -.-> Item11;
    Item16 -.-> Item9;
    Item17 --> Item14;
    Item17 -.-> Item16;
```
# Phase 3
```mermaid
graph TD
    Item3;
    Item3["ModuleEvaluation"];
    Item4;
    Item4["export external1"];
    Item5;
    Item5["export external2"];
    Item6;
    Item6["export foo"];
    Item7;
    Item7["export foobar"];
    Item8;
    Item1;
    Item9;
    Item10;
    Item11;
    Item12;
    Item13;
    Item14;
    Item15;
    Item16;
    Item17;
    Item18;
    Item19;
    Item2;
    Item2 --> Item1;
    Item4 --> Item8;
    Item5 --> Item19;
    Item10 -.-> Item7;
    Item11 --> Item10;
    Item11 -.-> Item6;
    Item13 --> Item12;
    Item13 --> Item10;
    Item13 -.-> Item7;
    Item13 -.-> Item11;
    Item14 --> Item13;
    Item14 --> Item10;
    Item15 --> Item13;
    Item15 --> Item10;
    Item15 -.-> Item7;
    Item15 -.-> Item11;
    Item15 -.-> Item14;
    Item16 --> Item14;
    Item16 --> Item1;
    Item16 --> Item2;
    Item16 -.-> Item18;
    Item16 -.-> Item15;
    Item16 -.-> Item7;
    Item16 -.-> Item11;
    Item16 -.-> Item9;
    Item17 --> Item14;
    Item17 -.-> Item16;
    Item8 --> Item18;
    Item8 --> Item15;
    Item18 --> Item9;
    Item18 --> Item15;
    Item19 -.-> Item7;
    Item19 -.-> Item11;
    Item19 -.-> Item14;
    Item19 -.-> Item15;
```
# Phase 4
```mermaid
graph TD
    Item3;
    Item3["ModuleEvaluation"];
    Item4;
    Item4["export external1"];
    Item5;
    Item5["export external2"];
    Item6;
    Item6["export foo"];
    Item7;
    Item7["export foobar"];
    Item8;
    Item1;
    Item9;
    Item10;
    Item11;
    Item12;
    Item13;
    Item14;
    Item15;
    Item16;
    Item17;
    Item18;
    Item19;
    Item2;
    Item2 --> Item1;
    Item4 --> Item8;
    Item5 --> Item19;
    Item10 -.-> Item7;
    Item11 --> Item10;
    Item11 -.-> Item6;
    Item13 --> Item12;
    Item13 --> Item10;
    Item13 -.-> Item7;
    Item13 -.-> Item11;
    Item14 --> Item13;
    Item14 --> Item10;
    Item15 --> Item13;
    Item15 --> Item10;
    Item15 -.-> Item7;
    Item15 -.-> Item11;
    Item15 -.-> Item14;
    Item16 --> Item14;
    Item16 --> Item1;
    Item16 --> Item2;
    Item16 -.-> Item18;
    Item16 -.-> Item15;
    Item16 -.-> Item7;
    Item16 -.-> Item11;
    Item16 -.-> Item9;
    Item17 --> Item14;
    Item17 -.-> Item16;
    Item8 --> Item18;
    Item8 --> Item15;
    Item18 --> Item9;
    Item18 --> Item15;
    Item19 -.-> Item7;
    Item19 -.-> Item11;
    Item19 -.-> Item14;
    Item19 -.-> Item15;
    Item3 --> Item1;
    Item3 --> Item2;
    Item3 --> Item16;
    Item6 --> Item11;
    Item7 --> Item15;
```
# Final
```mermaid
graph TD
    N0["Items: [ItemId(ModuleEvaluation), ItemId(1, ImportOfModule), ItemId(1, ImportBinding(0)), ItemId(8, Normal), ItemId(12, ImportOfModule)]"];
    N1["Items: [ItemId(Export((&quot;external1&quot;, #2), &quot;external1&quot;)), ItemId(0, Normal)]"];
    N2["Items: [ItemId(Export((&quot;external2&quot;, #2), &quot;external2&quot;)), ItemId(11, Normal)]"];
    N3["Items: [ItemId(Export((&quot;foo&quot;, #2), &quot;foo&quot;))]"];
    N4["Items: [ItemId(Export((&quot;foobar&quot;, #2), &quot;foobar&quot;))]"];
    N5["Items: [ItemId(2, VarDeclarator(0))]"];
    N6["Items: [ItemId(3, VarDeclarator(0))]"];
    N7["Items: [ItemId(4, VarDeclarator(0)), ItemId(5, Normal)]"];
    N8["Items: [ItemId(6, VarDeclarator(0))]"];
    N9["Items: [ItemId(7, Normal)]"];
    N10["Items: [ItemId(1, ImportBinding(0)), ItemId(10, Normal)]"];
    N0 --> N8;
    N0 --> N10;
    N0 --> N9;
    N0 --> N4;
    N0 --> N6;
    N1 --> N10;
    N1 --> N9;
    N2 --> N4;
    N2 --> N6;
    N2 --> N8;
    N2 --> N9;
    N3 --> N6;
    N4 --> N9;
    N5 --> N4;
    N6 --> N5;
    N6 --> N3;
    N7 --> N5;
    N7 --> N4;
    N7 --> N6;
    N8 --> N7;
    N8 --> N5;
    N9 --> N7;
    N9 --> N5;
    N9 --> N4;
    N9 --> N6;
    N9 --> N8;
    N10 --> N9;
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
    ): 4,
    Export(
        "external2",
    ): 2,
}
```


# Modules (dev)
## Part 0
```js
import { foobarCopy } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 8
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 10
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 9
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 4
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 6
};
"module evaluation";
import "module";
import { upper } from "module";
console.log(foobarCopy);
import "other";
export { upper } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 1
```js
import { internal } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 10
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 9
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
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 4
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 6
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 8
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 9
};
export { external2 };
function external2() {
    foobar += ".";
}
export { external2 } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 3
```js
import { foo } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 6
};
export { foo };

```
## Part 4
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 9
};
export { foobar };

```
## Part 5
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 4
};
let foobar = "foo";
export { foobar } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 6
```js
import { foobar } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 3
};
const foo = foobar;
export { foo } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 7
```js
import { foobar } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 4
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 6
};
const bar = "bar";
foobar += bar;
export { bar } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 8
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 7
};
import { foobar } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
let foobarCopy = foobar;
export { foobarCopy } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 9
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 7
};
import { foobar } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 4
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 6
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 8
};
foobar += "foo";

```
## Part 10
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 9
};
import { upper } from "module";
function internal() {
    return upper(foobar);
}
export { upper } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { internal } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Merged (module eval)
```js
import { foobarCopy } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 8
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 10
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 9
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 4
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 6
};
import "module";
import { upper } from "module";
import "other";
"module evaluation";
console.log(foobarCopy);
export { upper } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

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
    ): 4,
    Export(
        "external2",
    ): 2,
}
```


# Modules (prod)
## Part 0
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 6
};
import { foobar } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
"module evaluation";
import "module";
let foobarCopy = foobar;
console.log(foobarCopy);
import "other";
export { foobarCopy } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 1
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 7
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
export { upper } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { internal } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 2
```js
export { external2 };
function external2() {
    foobar += ".";
}
export { external2 } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 3
```js
import { foobar } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
export { foo };
const foo = foobar;
export { foo } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 4
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 7
};
export { foobar };

```
## Part 5
```js
let foobar = "foo";
export { foobar } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 6
```js
import { foobar } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
const bar = "bar";
foobar += bar;
export { bar } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 7
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 6
};
import { foobar } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
foobar += "foo";

```
## Merged (module eval)
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 6
};
import { foobar } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
import "module";
import "other";
"module evaluation";
let foobarCopy = foobar;
console.log(foobarCopy);
export { foobarCopy } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
