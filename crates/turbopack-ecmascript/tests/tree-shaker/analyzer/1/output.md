# Items

Count: 18

## Item 1: Stmt 0, `ImportOfModule`

```js
import { upper } from "module";

```

- Hoisted
- Side effects

## Item 2: Stmt 0, `ImportBinding(0)`

```js
import { upper } from "module";

```

- Hoisted
- Declares: `upper`

## Item 3: Stmt 1, `VarDeclarator(0)`

```js
export let foobar = "foo";

```

- Declares: `foobar`
- Write: `foobar`

## Item 4: Stmt 2, `VarDeclarator(0)`

```js
export const foo = foobar;

```

- Declares: `foo`
- Reads: `foobar`
- Write: `foo`

## Item 5: Stmt 3, `VarDeclarator(0)`

```js
const bar = "bar";

```

- Declares: `bar`
- Write: `bar`

## Item 6: Stmt 4, `Normal`

```js
foobar += bar;

```

- Side effects
- Reads: `bar`
- Write: `foobar`

## Item 7: Stmt 5, `VarDeclarator(0)`

```js
let foobarCopy = foobar;

```

- Declares: `foobarCopy`
- Reads: `foobar`
- Write: `foobarCopy`

## Item 8: Stmt 6, `Normal`

```js
foobar += "foo";

```

- Side effects
- Write: `foobar`

## Item 9: Stmt 7, `Normal`

```js
console.log(foobarCopy);

```

- Side effects
- Reads: `console`, `foobarCopy`

## Item 10: Stmt 8, `Normal`

```js
foobarCopy += "Unused";

```

- Side effects
- Write: `foobarCopy`

## Item 11: Stmt 9, `Normal`

```js
function internal() {
    return upper(foobar);
}

```

- Hoisted
- Declares: `internal`
- Reads (eventual): `upper`, `foobar`
- Write: `internal`

## Item 12: Stmt 10, `Normal`

```js
export function external1() {
    return internal() + foobar;
}

```

- Hoisted
- Declares: `external1`
- Reads (eventual): `internal`, `foobar`
- Write: `external1`

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
    Item12;
    Item13;
    Item14;
    Item14["ModuleEvaluation"];
    Item15;
    Item15["export foobar"];
    Item16;
    Item16["export foo"];
    Item17;
    Item17["export external1"];
    Item18;
    Item18["export external2"];
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
    Item12;
    Item13;
    Item14;
    Item14["ModuleEvaluation"];
    Item15;
    Item15["export foobar"];
    Item16;
    Item16["export foo"];
    Item17;
    Item17["export external1"];
    Item18;
    Item18["export external2"];
    Item4 --> Item3;
    Item6 --> Item5;
    Item6 -.-> Item4;
    Item6 --> Item1;
    Item6 -.-> Item2;
    Item6 -.-> Item3;
    Item6 -.-> Item11;
    Item7 --> Item3;
    Item7 --> Item6;
    Item8 -.-> Item4;
    Item8 -.-> Item7;
    Item8 --> Item1;
    Item8 --> Item6;
    Item8 -.-> Item2;
    Item8 -.-> Item3;
    Item8 -.-> Item11;
    Item9 --> Item7;
    Item9 --> Item1;
    Item9 --> Item6;
    Item9 --> Item8;
    Item9 -.-> Item2;
    Item9 -.-> Item3;
    Item9 -.-> Item4;
    Item9 -.-> Item11;
    Item10 --> Item9;
    Item10 --> Item1;
    Item10 --> Item6;
    Item10 --> Item8;
    Item10 -.-> Item2;
    Item10 -.-> Item3;
    Item10 -.-> Item4;
    Item10 -.-> Item7;
    Item10 -.-> Item11;
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
    Item12;
    Item13;
    Item14;
    Item14["ModuleEvaluation"];
    Item15;
    Item15["export foobar"];
    Item16;
    Item16["export foo"];
    Item17;
    Item17["export external1"];
    Item18;
    Item18["export external2"];
    Item4 --> Item3;
    Item6 --> Item5;
    Item6 -.-> Item4;
    Item6 --> Item1;
    Item6 -.-> Item2;
    Item6 -.-> Item3;
    Item6 -.-> Item11;
    Item7 --> Item3;
    Item7 --> Item6;
    Item8 -.-> Item4;
    Item8 -.-> Item7;
    Item8 --> Item1;
    Item8 --> Item6;
    Item8 -.-> Item2;
    Item8 -.-> Item3;
    Item8 -.-> Item11;
    Item9 --> Item7;
    Item9 --> Item1;
    Item9 --> Item6;
    Item9 --> Item8;
    Item9 -.-> Item2;
    Item9 -.-> Item3;
    Item9 -.-> Item4;
    Item9 -.-> Item11;
    Item10 --> Item9;
    Item10 --> Item1;
    Item10 --> Item6;
    Item10 --> Item8;
    Item10 -.-> Item2;
    Item10 -.-> Item3;
    Item10 -.-> Item4;
    Item10 -.-> Item7;
    Item10 -.-> Item11;
    Item11 --> Item2;
    Item11 --> Item3;
    Item11 --> Item8;
    Item12 --> Item11;
    Item12 --> Item3;
    Item12 --> Item8;
    Item13 -.-> Item4;
    Item13 -.-> Item7;
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
    Item12;
    Item13;
    Item14;
    Item14["ModuleEvaluation"];
    Item15;
    Item15["export foobar"];
    Item16;
    Item16["export foo"];
    Item17;
    Item17["export external1"];
    Item18;
    Item18["export external2"];
    Item4 --> Item3;
    Item6 --> Item5;
    Item6 -.-> Item4;
    Item6 --> Item1;
    Item6 -.-> Item2;
    Item6 -.-> Item3;
    Item6 -.-> Item11;
    Item7 --> Item3;
    Item7 --> Item6;
    Item8 -.-> Item4;
    Item8 -.-> Item7;
    Item8 --> Item1;
    Item8 --> Item6;
    Item8 -.-> Item2;
    Item8 -.-> Item3;
    Item8 -.-> Item11;
    Item9 --> Item7;
    Item9 --> Item1;
    Item9 --> Item6;
    Item9 --> Item8;
    Item9 -.-> Item2;
    Item9 -.-> Item3;
    Item9 -.-> Item4;
    Item9 -.-> Item11;
    Item10 --> Item9;
    Item10 --> Item1;
    Item10 --> Item6;
    Item10 --> Item8;
    Item10 -.-> Item2;
    Item10 -.-> Item3;
    Item10 -.-> Item4;
    Item10 -.-> Item7;
    Item10 -.-> Item11;
    Item11 --> Item2;
    Item11 --> Item3;
    Item11 --> Item8;
    Item12 --> Item11;
    Item12 --> Item3;
    Item12 --> Item8;
    Item13 -.-> Item4;
    Item13 -.-> Item7;
    Item14 --> Item1;
    Item14 --> Item6;
    Item14 --> Item8;
    Item14 --> Item9;
    Item14 --> Item10;
    Item15 --> Item3;
    Item15 --> Item8;
    Item16 --> Item4;
    Item17 --> Item12;
    Item18 --> Item13;
```
# Final
```mermaid
graph TD
    N0["Items: [ItemId(ModuleEvaluation), ItemId(0, ImportBinding(0)), ItemId(7, Normal), ItemId(8, Normal)]"];
    N1["Items: [ItemId(Export((&quot;foobar&quot;, #2), &quot;foobar&quot;))]"];
    N2["Items: [ItemId(Export((&quot;foo&quot;, #2), &quot;foo&quot;))]"];
    N3["Items: [ItemId(Export((&quot;external1&quot;, #2), &quot;external1&quot;)), ItemId(10, Normal)]"];
    N4["Items: [ItemId(Export((&quot;external2&quot;, #2), &quot;external2&quot;)), ItemId(11, Normal)]"];
    N5["Items: [ItemId(0, ImportBinding(0)), ItemId(4, Normal), ItemId(5, VarDeclarator(0)), ItemId(6, Normal), ItemId(9, Normal)]"];
    N6["Items: [ItemId(0, ImportOfModule)]"];
    N7["Items: [ItemId(1, VarDeclarator(0))]"];
    N8["Items: [ItemId(2, VarDeclarator(0))]"];
    N9["Items: [ItemId(3, VarDeclarator(0))]"];
    N0 --> N6;
    N0 --> N5;
    N0 --> N7;
    N0 --> N8;
    N1 --> N7;
    N1 --> N5;
    N2 --> N8;
    N3 --> N5;
    N3 --> N7;
    N4 --> N8;
    N4 --> N5;
    N5 --> N9;
    N5 --> N8;
    N5 --> N6;
    N5 --> N7;
    N8 --> N7;
```
# Entrypoints

```
{
    ModuleEvaluation: 0,
    Export(
        "external1",
    ): 3,
    Export(
        "foo",
    ): 2,
    Export(
        "foobar",
    ): 1,
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
import { foobarCopy } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 7
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 8
};
"module evaluation";
import { upper } from "module";
console.log(foobarCopy);
foobarCopy += "Unused";

```
## Part 1
```js
import { foobar } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 7
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
export { foobar };

```
## Part 2
```js
import { foo } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 8
};
export { foo };

```
## Part 3
```js
import { foobar, internal } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 7
};
export { external1 };
function external1() {
    return internal() + foobar;
}
export { external1 } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 4
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 8
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
    __turbopack_part__: 9
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 8
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 6
};
import { foobar } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 7
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
let foobar = "foo";
export { foobar } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 8
```js
import { foobar } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 7
};
const foo = foobar;
export { foo } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 9
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
import { foobarCopy } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 7
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 8
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
    ): 3,
    Export(
        "foo",
    ): 2,
    Export(
        "foobar",
    ): 1,
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
import { foobar } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 8
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 9
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 6
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
    __turbopack_part__: 6
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 9
};
export { foobar };

```
## Part 2
```js
import { foobar } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 6
};
export { foo };
const foo = foobar;
export { foo } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 3
```js
import { foobar } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 9
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 6
};
export { external1 };
import { upper } from "module";
function internal() {
    return upper(foobar);
}
function external1() {
    return internal() + foobar;
}
export { internal } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { external1 } from "__TURBOPACK_VAR__" assert {
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
let foobar = "foo";
export { foobar } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 7
```js
const bar = "bar";
export { bar } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 8
```js
import { bar } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 7
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
foobar += bar;
export { foobar } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 9
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
import { foobar } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 8
};
foobar += "foo";

```
## Merged (module eval)
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
import { foobar } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 8
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 9
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 6
};
"module evaluation";
let foobarCopy = foobar;
console.log(foobarCopy);
foobarCopy += "Unused";
export { foobarCopy } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
