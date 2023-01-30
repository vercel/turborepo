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

- Side effects
- Declares: "`foobar`"
- Write: "`foobar`"

## Item 4: Stmt 2, `VarDeclarator(0)`

```js
export const foo = foobar;
```

- Side effects
- Declares: "`foo`"
- Reads: "`foobar`"
- Write: "`foo`"

## Item 5: Stmt 3, `VarDeclarator(0)`

```js
const bar = "bar";
```

- Side effects
- Declares: "`bar`"
- Write: "`bar`"

## Item 6: Stmt 4, `Normal`

```js
foobar += bar;
```

- Side effects
- Reads: "`bar`"
- Write: "`foobar`"

## Item 7: Stmt 5, `VarDeclarator(0)`

```js
let foobarCopy = foobar;
```

- Side effects
- Declares: "`foobarCopy`"
- Reads: "`foobar`"
- Write: "`foobarCopy`"

## Item 8: Stmt 6, `Normal`

```js
foobar += "foo";
```

- Side effects
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

- Side effects
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
    Item3 --> Item1;
    Item3 -.-> Item2;
    Item3 -.-> Item11;
    Item4 --> Item3;
    Item4 -.-> Item2;
    Item4 -.-> Item11;
    Item5 --> Item4;
    Item5 -.-> Item2;
    Item5 -.-> Item3;
    Item5 -.-> Item11;
    Item6 --> Item5;
    Item6 -.-> Item4;
    Item6 -.-> Item2;
    Item6 -.-> Item3;
    Item6 -.-> Item11;
    Item7 --> Item3;
    Item7 --> Item6;
    Item7 -.-> Item2;
    Item7 -.-> Item4;
    Item7 -.-> Item11;
    Item8 -.-> Item4;
    Item8 --> Item7;
    Item8 -.-> Item2;
    Item8 -.-> Item3;
    Item8 -.-> Item6;
    Item8 -.-> Item11;
    Item9 --> Item7;
    Item9 --> Item8;
    Item9 -.-> Item2;
    Item9 -.-> Item3;
    Item9 -.-> Item6;
    Item9 -.-> Item4;
    Item9 -.-> Item11;
    Item10 --> Item9;
    Item10 -.-> Item2;
    Item10 -.-> Item3;
    Item10 -.-> Item6;
    Item10 -.-> Item8;
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
    Item3 --> Item1;
    Item3 -.-> Item2;
    Item3 -.-> Item11;
    Item4 --> Item3;
    Item4 -.-> Item2;
    Item4 -.-> Item11;
    Item5 --> Item4;
    Item5 -.-> Item2;
    Item5 -.-> Item3;
    Item5 -.-> Item11;
    Item6 --> Item5;
    Item6 -.-> Item4;
    Item6 -.-> Item2;
    Item6 -.-> Item3;
    Item6 -.-> Item11;
    Item7 --> Item3;
    Item7 --> Item6;
    Item7 -.-> Item2;
    Item7 -.-> Item4;
    Item7 -.-> Item11;
    Item8 -.-> Item4;
    Item8 --> Item7;
    Item8 -.-> Item2;
    Item8 -.-> Item3;
    Item8 -.-> Item6;
    Item8 -.-> Item11;
    Item9 --> Item7;
    Item9 --> Item8;
    Item9 -.-> Item2;
    Item9 -.-> Item3;
    Item9 -.-> Item6;
    Item9 -.-> Item4;
    Item9 -.-> Item11;
    Item10 --> Item9;
    Item10 -.-> Item2;
    Item10 -.-> Item3;
    Item10 -.-> Item6;
    Item10 -.-> Item8;
    Item10 -.-> Item4;
    Item10 -.-> Item7;
    Item10 -.-> Item11;
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
    Item3 --> Item1;
    Item3 -.-> Item2;
    Item3 -.-> Item11;
    Item4 --> Item3;
    Item4 -.-> Item2;
    Item4 -.-> Item11;
    Item5 --> Item4;
    Item5 -.-> Item2;
    Item5 -.-> Item3;
    Item5 -.-> Item11;
    Item6 --> Item5;
    Item6 -.-> Item4;
    Item6 -.-> Item2;
    Item6 -.-> Item3;
    Item6 -.-> Item11;
    Item7 --> Item3;
    Item7 --> Item6;
    Item7 -.-> Item2;
    Item7 -.-> Item4;
    Item7 -.-> Item11;
    Item8 -.-> Item4;
    Item8 --> Item7;
    Item8 -.-> Item2;
    Item8 -.-> Item3;
    Item8 -.-> Item6;
    Item8 -.-> Item11;
    Item9 --> Item7;
    Item9 --> Item8;
    Item9 -.-> Item2;
    Item9 -.-> Item3;
    Item9 -.-> Item6;
    Item9 -.-> Item4;
    Item9 -.-> Item11;
    Item10 --> Item9;
    Item10 -.-> Item2;
    Item10 -.-> Item3;
    Item10 -.-> Item6;
    Item10 -.-> Item8;
    Item10 -.-> Item4;
    Item10 -.-> Item7;
    Item10 -.-> Item11;
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
    Item14 --> Item10;
    Item14 -.-> Item12;
    Item14 -.-> Item9;
    Item14 -.-> Item2;
    Item14 -.-> Item3;
    Item14 -.-> Item6;
    Item14 -.-> Item8;
    Item14 -.-> Item4;
    Item14 -.-> Item7;
    Item14 -.-> Item5;
    Item14 -.-> Item11;
    Item14 -.-> Item13;
    Item15 --> Item3;
    Item15 --> Item6;
    Item15 --> Item8;
    Item16 --> Item4;
    Item17 --> Item12;
    Item18 --> Item13;
```
