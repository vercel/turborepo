# Items

Count: 19

## Item 1: Stmt 0, `Normal`

```js
export function external1() {
  return internal() + foobar;
}
```

- Hoisted
- Declares: "`external1`"
- Reads (eventual): "`internal`, `foobar`"

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
- Declares: "`upper`"

## Item 4: Stmt 2, `VarDeclarator(0)`

```js
export let foobar = "foo";
```

- Declares: "`foobar`"
- Write: "`foobar`"

## Item 5: Stmt 3, `VarDeclarator(0)`

```js
export const foo = foobar;
```

- Declares: "`foo`"
- Reads: "`foobar`"
- Write: "`foo`"

## Item 6: Stmt 4, `VarDeclarator(0)`

```js
const bar = "bar";
```

- Declares: "`bar`"
- Write: "`bar`"

## Item 7: Stmt 5, `Normal`

```js
foobar += bar;
```

- Reads: "`bar`, `foobar`"
- Write: "`foobar`"

## Item 8: Stmt 6, `VarDeclarator(0)`

```js
let foobarCopy = foobar;
```

- Declares: "`foobarCopy`"
- Reads: "`foobar`"
- Write: "`foobarCopy`"

## Item 9: Stmt 7, `Normal`

```js
foobar += "foo";
```

- Reads: "`foobar`"
- Write: "`foobar`"

## Item 10: Stmt 8, `Normal`

```js
console.log(foobarCopy);
```

- Side effects
- Reads: "`console`, `foobarCopy`"

## Item 11: Stmt 9, `Normal`

```js
foobarCopy += "Unused";
```

- Reads: "`foobarCopy`"
- Write: "`foobarCopy`"

## Item 12: Stmt 10, `Normal`

```js
function internal() {
  return upper(foobar);
}
```

- Hoisted
- Declares: "`internal`"
- Reads (eventual): "`upper`, `foobar`"

## Item 13: Stmt 11, `Normal`

```js
export function external2() {
  foobar += ".";
}
```

- Hoisted
- Declares: "`external2`"
- Write (eventual): "`foobar`"

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
    Item2;
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
    Item1;
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
    Item1 --> Item2;
```

# Phase 2

```mermaid
graph TD
    Item3;
    Item2;
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
    Item1;
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
    Item1 --> Item2;
    Item6 --> Item5;
    Item8 --> Item7;
    Item8 --> Item5;
    Item8 -.-> Item6;
    Item9 --> Item5;
    Item9 --> Item8;
    Item10 --> Item5;
    Item10 --> Item8;
    Item10 -.-> Item6;
    Item10 -.-> Item9;
    Item11 --> Item9;
    Item11 --> Item1;
    Item11 -.-> Item13;
    Item11 -.-> Item5;
    Item11 -.-> Item8;
    Item11 -.-> Item10;
    Item11 -.-> Item6;
    Item11 -.-> Item4;
    Item12 --> Item9;
    Item12 -.-> Item11;
```

# Phase 3

```mermaid
graph TD
    Item3;
    Item2;
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
    Item1;
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
    Item1 --> Item2;
    Item6 --> Item5;
    Item8 --> Item7;
    Item8 --> Item5;
    Item8 -.-> Item6;
    Item9 --> Item5;
    Item9 --> Item8;
    Item10 --> Item5;
    Item10 --> Item8;
    Item10 -.-> Item6;
    Item10 -.-> Item9;
    Item11 --> Item9;
    Item11 --> Item1;
    Item11 -.-> Item13;
    Item11 -.-> Item5;
    Item11 -.-> Item8;
    Item11 -.-> Item10;
    Item11 -.-> Item6;
    Item11 -.-> Item4;
    Item12 --> Item9;
    Item12 -.-> Item11;
    Item3 --> Item13;
    Item3 --> Item5;
    Item3 --> Item8;
    Item3 --> Item10;
    Item13 --> Item4;
    Item13 --> Item5;
    Item13 --> Item8;
    Item13 --> Item10;
    Item14 -.-> Item6;
    Item14 -.-> Item9;
    Item14 -.-> Item10;
```

# Phase 4

```mermaid
graph TD
    Item3;
    Item2;
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
    Item1;
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
    Item1 --> Item2;
    Item6 --> Item5;
    Item8 --> Item7;
    Item8 --> Item5;
    Item8 -.-> Item6;
    Item9 --> Item5;
    Item9 --> Item8;
    Item10 --> Item5;
    Item10 --> Item8;
    Item10 -.-> Item6;
    Item10 -.-> Item9;
    Item11 --> Item9;
    Item11 --> Item1;
    Item11 -.-> Item13;
    Item11 -.-> Item5;
    Item11 -.-> Item8;
    Item11 -.-> Item10;
    Item11 -.-> Item6;
    Item11 -.-> Item4;
    Item12 --> Item9;
    Item12 -.-> Item11;
    Item3 --> Item13;
    Item3 --> Item5;
    Item3 --> Item8;
    Item3 --> Item10;
    Item13 --> Item4;
    Item13 --> Item5;
    Item13 --> Item8;
    Item13 --> Item10;
    Item14 -.-> Item6;
    Item14 -.-> Item9;
    Item14 -.-> Item10;
    Item15 --> Item2;
    Item15 --> Item1;
    Item15 --> Item11;
    Item16 --> Item3;
    Item17 --> Item5;
    Item17 --> Item8;
    Item17 --> Item10;
    Item18 --> Item6;
    Item19 --> Item14;
```

# Final

```mermaid
graph TD
    N0["Items: [ItemId(1, ImportOfModule), ItemId(8, Normal), ItemId(10, Normal), ItemId(12, ImportOfModule), ItemId(ModuleEvaluation)]"];
    N1["Items: [ItemId(Export((Atom('external1' type=dynamic), #0)))]"];
    N2["Items: [ItemId(Export((Atom('foobar' type=inline), #0)))]"];
    N3["Items: [ItemId(Export((Atom('foo' type=inline), #0)))]"];
    N4["Items: [ItemId(Export((Atom('external2' type=dynamic), #0)))]"];
    N5["Items: [ItemId(0, Normal)]"];
    N6["Items: [ItemId(1, ImportBinding(0))]"];
    N7["Items: [ItemId(2, VarDeclarator(0))]"];
    N8["Items: [ItemId(3, VarDeclarator(0))]"];
    N9["Items: [ItemId(5, Normal)]"];
    N10["Items: [ItemId(6, VarDeclarator(0))]"];
    N11["Items: [ItemId(9, Normal)]"];
    N12["Items: [ItemId(11, Normal)]"];
    N13["Items: [ItemId(4, VarDeclarator(0))]"];
    N14["Items: [ItemId(7, Normal)]"];
    N0 --> N10;
    N0 --> N7;
    N0 --> N9;
    N0 --> N14;
    N0 --> N8;
    N0 --> N6;
    N1 --> N5;
    N2 --> N7;
    N2 --> N9;
    N2 --> N14;
    N3 --> N8;
    N4 --> N12;
    N5 --> N0;
    N5 --> N7;
    N5 --> N9;
    N5 --> N14;
    N8 --> N7;
    N9 --> N13;
    N9 --> N7;
    N9 --> N8;
    N10 --> N7;
    N10 --> N9;
    N11 --> N10;
    N11 --> N0;
    N12 --> N8;
    N12 --> N10;
    N12 --> N14;
    N14 --> N7;
    N14 --> N9;
    N14 --> N8;
    N14 --> N10;
```

# Modules (dev)

## Module 1

```js
"turbopack://chunk-0";
import "turbopack://chunk-10.js";
import "turbopack://chunk-7.js";
import "turbopack://chunk-9.js";
import "turbopack://chunk-14.js";
import "turbopack://chunk-8.js";
import "turbopack://chunk-6.js";
import "module";
console.log(foobarCopy);
function internal() {
  return upper(foobar);
}
import "other";
("module evaluation");
```

## Module 2

```js
"turbopack://chunk-1";
import "turbopack://chunk-5.js";
export { external1 };
```

## Module 3

```js
"turbopack://chunk-2";
import "turbopack://chunk-7.js";
import "turbopack://chunk-9.js";
import "turbopack://chunk-14.js";
export { foobar };
```

## Module 4

```js
"turbopack://chunk-3";
import "turbopack://chunk-8.js";
export { foo };
```

## Module 5

```js
"turbopack://chunk-4";
import "turbopack://chunk-12.js";
export { external2 };
```

## Module 6

```js
"turbopack://chunk-5";
import "turbopack://chunk-0.js";
import "turbopack://chunk-7.js";
import "turbopack://chunk-9.js";
import "turbopack://chunk-14.js";
export function external1() {
  return internal() + foobar;
}
```

## Module 7

```js
"turbopack://chunk-6";
import { upper } from "module";
```

## Module 8

```js
"turbopack://chunk-7";
export let foobar = "foo";
```

## Module 9

```js
"turbopack://chunk-8";
import "turbopack://chunk-7.js";
export const foo = foobar;
```

## Module 10

```js
"turbopack://chunk-9";
import "turbopack://chunk-13.js";
import "turbopack://chunk-7.js";
import "turbopack://chunk-8.js";
foobar += bar;
```

## Module 11

```js
"turbopack://chunk-10";
import "turbopack://chunk-7.js";
import "turbopack://chunk-9.js";
let foobarCopy = foobar;
```

## Module 12

```js
"turbopack://chunk-11";
import "turbopack://chunk-10.js";
import "turbopack://chunk-0.js";
foobarCopy += "Unused";
```

## Module 13

```js
"turbopack://chunk-12";
import "turbopack://chunk-8.js";
import "turbopack://chunk-10.js";
import "turbopack://chunk-14.js";
export function external2() {
  foobar += ".";
}
```

## Module 14

```js
"turbopack://chunk-13";
const bar = "bar";
```

## Module 15

```js
"turbopack://chunk-14";
import "turbopack://chunk-7.js";
import "turbopack://chunk-9.js";
import "turbopack://chunk-8.js";
import "turbopack://chunk-10.js";
foobar += "foo";
```

# Modules (prod)

## Module 1

```js
"turbopack://chunk-0";
import "turbopack://chunk-10.js";
import "module";
import "other";
("module evaluation");
```

## Module 2

```js
"turbopack://chunk-1";
import "turbopack://chunk-5.js";
export { external1 };
```

## Module 3

```js
"turbopack://chunk-2";
import "turbopack://chunk-6.js";
import "turbopack://chunk-8.js";
import "turbopack://chunk-16.js";
export { foobar };
```

## Module 4

```js
"turbopack://chunk-3";
import "turbopack://chunk-7.js";
export { foo };
```

## Module 5

```js
"turbopack://chunk-4";
import "turbopack://chunk-13.js";
export { external2 };
```

## Module 6

```js
"turbopack://chunk-5";
import "turbopack://chunk-12.js";
import "turbopack://chunk-6.js";
import "turbopack://chunk-8.js";
import "turbopack://chunk-16.js";
export function external1() {
  return internal() + foobar;
}
```

## Module 7

```js
"turbopack://chunk-6";
export let foobar = "foo";
```

## Module 8

```js
"turbopack://chunk-7";
import "turbopack://chunk-6.js";
export const foo = foobar;
```

## Module 9

```js
"turbopack://chunk-8";
import "turbopack://chunk-15.js";
import "turbopack://chunk-6.js";
foobar += bar;
```

## Module 10

```js
"turbopack://chunk-9";
import "turbopack://chunk-6.js";
import "turbopack://chunk-8.js";
let foobarCopy = foobar;
```

## Module 11

```js
"turbopack://chunk-10";
import "turbopack://chunk-9.js";
import "turbopack://chunk-0.js";
console.log(foobarCopy);
```

## Module 12

```js
"turbopack://chunk-11";
import "turbopack://chunk-9.js";
foobarCopy += "Unused";
```

## Module 13

```js
"turbopack://chunk-12";
import "turbopack://chunk-16.js";
import "turbopack://chunk-14.js";
import "turbopack://chunk-6.js";
import "turbopack://chunk-8.js";
function internal() {
  return upper(foobar);
}
```

## Module 14

```js
"turbopack://chunk-13";
export function external2() {
  foobar += ".";
}
```

## Module 15

```js
"turbopack://chunk-14";
import { upper } from "module";
```

## Module 16

```js
"turbopack://chunk-15";
const bar = "bar";
```

## Module 17

```js
"turbopack://chunk-16";
import "turbopack://chunk-6.js";
import "turbopack://chunk-8.js";
foobar += "foo";
```
