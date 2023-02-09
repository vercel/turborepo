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
    Item14 --> Item9;
    Item14 -.-> Item12;
    Item14 -.-> Item2;
    Item14 -.-> Item3;
    Item14 -.-> Item6;
    Item14 -.-> Item8;
    Item14 -.-> Item4;
    Item14 -.-> Item7;
    Item14 -.-> Item5;
    Item14 -.-> Item10;
    Item14 -.-> Item11;
    Item14 -.-> Item13;
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
    N0["Items: [ItemId(0, ImportBinding(0))]"];
    N1["Items: [ItemId(1, VarDeclarator(0))]"];
    N2["Items: [ItemId(2, VarDeclarator(0))]"];
    N3["Items: [ItemId(3, VarDeclarator(0))]"];
    N4["Items: [ItemId(4, Normal)]"];
    N5["Items: [ItemId(5, VarDeclarator(0))]"];
    N6["Items: [ItemId(6, Normal)]"];
    N7["Items: [ItemId(7, Normal), ItemId(0, ImportOfModule)]"];
    N8["Items: [ItemId(9, Normal)]"];
    N9["Items: [ItemId(10, Normal)]"];
    N10["Items: [ItemId(11, Normal)]"];
    N11["Items: [ItemId(ModuleEvaluation), ItemId(0, ImportBinding(0)), ItemId(3, VarDeclarator(0)), ItemId(8, Normal), ItemId(5, VarDeclarator(0)), ItemId(7, Normal), ItemId(9, Normal)]"];
    N12["Items: [ItemId(Export((Atom('foobar' type=inline), #0))), ItemId(1, VarDeclarator(0)), ItemId(4, Normal), ItemId(6, Normal)]"];
    N13["Items: [ItemId(Export((Atom('foo' type=inline), #0))), ItemId(2, VarDeclarator(0))]"];
    N14["Items: [ItemId(Export((Atom('external1' type=dynamic), #0))), ItemId(10, Normal)]"];
    N15["Items: [ItemId(Export((Atom('external2' type=dynamic), #0))), ItemId(11, Normal)]"];
    N2 --> N12;
    N4 --> N11;
    N4 --> N12;
    N4 --> N13;
    N5 --> N12;
    N6 --> N12;
    N6 --> N13;
    N6 --> N11;
    N7 --> N11;
    N7 --> N7;
    N7 --> N12;
    N7 --> N13;
    N8 --> N11;
    N8 --> N12;
    N9 --> N11;
    N9 --> N12;
    N10 --> N13;
    N10 --> N11;
    N10 --> N12;
    N11 --> N11;
    N11 --> N14;
    N11 --> N12;
    N11 --> N13;
    N11 --> N15;
    N11 --> N7;
    N12 --> N12;
    N12 --> N11;
    N12 --> N13;
    N13 --> N13;
    N13 --> N12;
    N14 --> N14;
    N14 --> N11;
    N14 --> N12;
    N15 --> N15;
    N15 --> N13;
    N15 --> N11;
    N15 --> N12;
```
