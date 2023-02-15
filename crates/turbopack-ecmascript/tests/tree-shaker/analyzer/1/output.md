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
- Declares: "`upper`"

## Item 3: Stmt 1, `VarDeclarator(0)`

```js
export let foobar = "foo";
```

- Declares: "`foobar`"
- Write: "`foobar`"

## Item 4: Stmt 2, `VarDeclarator(0)`

```js
export const foo = foobar;
```

- Declares: "`foo`"
- Reads: "`foobar`"
- Write: "`foo`"

## Item 5: Stmt 3, `VarDeclarator(0)`

```js
const bar = "bar";
```

- Declares: "`bar`"
- Write: "`bar`"

## Item 6: Stmt 4, `Normal`

```js
foobar += bar;
```

- Reads: "`bar`, `foobar`"
- Write: "`foobar`"

## Item 7: Stmt 5, `VarDeclarator(0)`

```js
let foobarCopy = foobar;
```

- Declares: "`foobarCopy`"
- Reads: "`foobar`"
- Write: "`foobarCopy`"

## Item 8: Stmt 6, `Normal`

```js
foobar += "foo";
```

- Reads: "`foobar`"
- Write: "`foobar`"

## Item 9: Stmt 7, `Normal`

```js
console.log(foobarCopy);
```

- Side effects
- Reads: "`console`, `foobarCopy`"

## Item 10: Stmt 8, `Normal`

```js
foobarCopy += "Unused";
```

- Reads: "`foobarCopy`"
- Write: "`foobarCopy`"

## Item 11: Stmt 9, `Normal`

```js
function internal() {
  return upper(foobar);
}
```

- Hoisted
- Declares: "`internal`"
- Reads (eventual): "`upper`, `foobar`"

## Item 12: Stmt 10, `Normal`

```js
export function external1() {
  return internal() + foobar;
}
```

- Hoisted
- Declares: "`external1`"
- Reads (eventual): "`internal`, `foobar`"

## Item 13: Stmt 11, `Normal`

```js
export function external2() {
  foobar += ".";
}
```

- Hoisted
- Declares: "`external2`"
- Write (eventual): "`foobar`"

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
    Item6 --> Item3;
    Item6 -.-> Item4;
    Item7 --> Item3;
    Item7 --> Item6;
    Item8 --> Item3;
    Item8 --> Item6;
    Item8 -.-> Item4;
    Item8 -.-> Item7;
    Item9 --> Item7;
    Item9 --> Item1;
    Item9 -.-> Item2;
    Item9 -.-> Item3;
    Item9 -.-> Item6;
    Item9 -.-> Item8;
    Item9 -.-> Item4;
    Item9 -.-> Item11;
    Item10 --> Item7;
    Item10 -.-> Item9;
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
    Item6 --> Item3;
    Item6 -.-> Item4;
    Item7 --> Item3;
    Item7 --> Item6;
    Item8 --> Item3;
    Item8 --> Item6;
    Item8 -.-> Item4;
    Item8 -.-> Item7;
    Item9 --> Item7;
    Item9 --> Item1;
    Item9 -.-> Item2;
    Item9 -.-> Item3;
    Item9 -.-> Item6;
    Item9 -.-> Item8;
    Item9 -.-> Item4;
    Item9 -.-> Item11;
    Item10 --> Item7;
    Item10 -.-> Item9;
    Item11 --> Item2;
    Item11 --> Item3;
    Item11 --> Item6;
    Item11 --> Item8;
    Item12 --> Item11;
    Item12 --> Item3;
    Item12 --> Item6;
    Item12 --> Item8;
    Item13 -.-> Item4;
    Item13 -.-> Item7;
    Item13 -.-> Item8;
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
    Item6 --> Item3;
    Item6 -.-> Item4;
    Item7 --> Item3;
    Item7 --> Item6;
    Item8 --> Item3;
    Item8 --> Item6;
    Item8 -.-> Item4;
    Item8 -.-> Item7;
    Item9 --> Item7;
    Item9 --> Item1;
    Item9 -.-> Item2;
    Item9 -.-> Item3;
    Item9 -.-> Item6;
    Item9 -.-> Item8;
    Item9 -.-> Item4;
    Item9 -.-> Item11;
    Item10 --> Item7;
    Item10 -.-> Item9;
    Item11 --> Item2;
    Item11 --> Item3;
    Item11 --> Item6;
    Item11 --> Item8;
    Item12 --> Item11;
    Item12 --> Item3;
    Item12 --> Item6;
    Item12 --> Item8;
    Item13 -.-> Item4;
    Item13 -.-> Item7;
    Item13 -.-> Item8;
    Item14 --> Item1;
    Item14 --> Item9;
    Item15 --> Item3;
    Item15 --> Item6;
    Item15 --> Item8;
    Item16 --> Item4;
    Item17 --> Item12;
    Item18 --> Item13;
```

# Final

```mermaid
graph TD
    N0["Items: [ItemId(0, ImportOfModule), ItemId(7, Normal), ItemId(ModuleEvaluation)]"];
    N1["Items: [ItemId(Export((Atom('foobar' type=inline), #0)))]"];
    N2["Items: [ItemId(Export((Atom('foo' type=inline), #0)))]"];
    N3["Items: [ItemId(10, Normal), ItemId(Export((Atom('external1' type=dynamic), #0)))]"];
    N4["Items: [ItemId(11, Normal), ItemId(Export((Atom('external2' type=dynamic), #0)))]"];
    N5["Items: [ItemId(0, ImportBinding(0))]"];
    N6["Items: [ItemId(1, VarDeclarator(0))]"];
    N7["Items: [ItemId(2, VarDeclarator(0))]"];
    N8["Items: [ItemId(3, VarDeclarator(0))]"];
    N9["Items: [ItemId(4, Normal)]"];
    N10["Items: [ItemId(5, VarDeclarator(0))]"];
    N11["Items: [ItemId(6, Normal)]"];
    N12["Items: [ItemId(9, Normal)]"];
    N0 --> N10;
    N0 --> N5;
    N0 --> N6;
    N0 --> N9;
    N0 --> N11;
    N0 --> N7;
    N0 --> N12;
    N1 --> N6;
    N1 --> N9;
    N1 --> N11;
    N2 --> N7;
    N3 --> N12;
    N3 --> N6;
    N3 --> N9;
    N3 --> N11;
    N4 --> N7;
    N4 --> N10;
    N4 --> N11;
    N7 --> N6;
    N9 --> N8;
    N9 --> N6;
    N9 --> N7;
    N10 --> N6;
    N10 --> N9;
    N11 --> N6;
    N11 --> N9;
    N11 --> N7;
    N11 --> N10;
    N12 --> N5;
    N12 --> N6;
    N12 --> N9;
    N12 --> N11;
```

# Modules (dev)

## Module 1

```js
import "entry.js" assert {
    __turbopack_chunk__: 10
};
import "entry.js" assert {
    __turbopack_chunk__: 5
};
import "entry.js" assert {
    __turbopack_chunk__: 6
};
import "entry.js" assert {
    __turbopack_chunk__: 9
};
import "entry.js" assert {
    __turbopack_chunk__: 11
};
import "entry.js" assert {
    __turbopack_chunk__: 7
};
import "entry.js" assert {
    __turbopack_chunk__: 12
};
import "module";
console.log(foobarCopy);
"module evaluation";

```

## Module 2

```js
import "entry.js" assert {
    __turbopack_chunk__: 6
};
import "entry.js" assert {
    __turbopack_chunk__: 9
};
import "entry.js" assert {
    __turbopack_chunk__: 11
};
export { foobar };

```

## Module 3

```js
import "entry.js" assert {
    __turbopack_chunk__: 7
};
export { foo };

```

## Module 4

```js
import "entry.js" assert {
    __turbopack_chunk__: 12
};
import "entry.js" assert {
    __turbopack_chunk__: 6
};
import "entry.js" assert {
    __turbopack_chunk__: 9
};
import "entry.js" assert {
    __turbopack_chunk__: 11
};
export function external1() {
    return internal() + foobar;
}
export { external1 };

```

## Module 5

```js
import "entry.js" assert {
    __turbopack_chunk__: 7
};
import "entry.js" assert {
    __turbopack_chunk__: 10
};
import "entry.js" assert {
    __turbopack_chunk__: 11
};
export function external2() {
    foobar += ".";
}
export { external2 };

```

## Module 6

```js
import { upper } from "module";
```

## Module 7

```js
export let foobar = "foo";
```

## Module 8

```js
import "entry.js" assert {
    __turbopack_chunk__: 6
};
export const foo = foobar;

```

## Module 9

```js
const bar = "bar";
```

## Module 10

```js
import "entry.js" assert {
    __turbopack_chunk__: 8
};
import "entry.js" assert {
    __turbopack_chunk__: 6
};
import "entry.js" assert {
    __turbopack_chunk__: 7
};
foobar += bar;

```

## Module 11

```js
import "entry.js" assert {
    __turbopack_chunk__: 6
};
import "entry.js" assert {
    __turbopack_chunk__: 9
};
let foobarCopy = foobar;

```

## Module 12

```js
import "entry.js" assert {
    __turbopack_chunk__: 6
};
import "entry.js" assert {
    __turbopack_chunk__: 9
};
import "entry.js" assert {
    __turbopack_chunk__: 7
};
import "entry.js" assert {
    __turbopack_chunk__: 10
};
foobar += "foo";

```

## Module 13

```js
import "entry.js" assert {
    __turbopack_chunk__: 5
};
import "entry.js" assert {
    __turbopack_chunk__: 6
};
import "entry.js" assert {
    __turbopack_chunk__: 9
};
import "entry.js" assert {
    __turbopack_chunk__: 11
};
function internal() {
    return upper(foobar);
}

```

## Merged (module eval)

```js
export let foobar = "foo";
const bar = "bar";
export const foo = foobar;
foobar += bar;
let foobarCopy = foobar;
import { upper } from "module";
foobar += "foo";
function internal() {
  return upper(foobar);
}
import "module";
console.log(foobarCopy);
("module evaluation");
```

# Modules (prod)

## Module 1

```js
import "entry.js" assert {
    __turbopack_chunk__: 5
};
import "entry.js" assert {
    __turbopack_chunk__: 7
};
import "module";
let foobarCopy = foobar;
console.log(foobarCopy);
"module evaluation";

```

## Module 2

```js
import "entry.js" assert {
    __turbopack_chunk__: 5
};
import "entry.js" assert {
    __turbopack_chunk__: 7
};
import "entry.js" assert {
    __turbopack_chunk__: 8
};
export { foobar };

```

## Module 3

```js
import "entry.js" assert {
    __turbopack_chunk__: 5
};
export const foo = foobar;
export { foo };

```

## Module 4

```js
import "entry.js" assert {
    __turbopack_chunk__: 5
};
import "entry.js" assert {
    __turbopack_chunk__: 7
};
import "entry.js" assert {
    __turbopack_chunk__: 8
};
import { upper } from "module";
function internal() {
    return upper(foobar);
}
export function external1() {
    return internal() + foobar;
}
export { external1 };

```

## Module 5

```js
export function external2() {
  foobar += ".";
}
export { external2 };
```

## Module 6

```js
export let foobar = "foo";
```

## Module 7

```js
const bar = "bar";
```

## Module 8

```js
import "entry.js" assert {
    __turbopack_chunk__: 6
};
import "entry.js" assert {
    __turbopack_chunk__: 5
};
foobar += bar;

```

## Module 9

```js
import "entry.js" assert {
    __turbopack_chunk__: 5
};
import "entry.js" assert {
    __turbopack_chunk__: 7
};
foobar += "foo";

```

## Merged (module eval)

```js
export let foobar = "foo";
const bar = "bar";
foobar += bar;
import "module";
let foobarCopy = foobar;
console.log(foobarCopy);
("module evaluation");
```
