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
    Item15 --> Item11;
    Item15 -.-> Item3;
    Item15 -.-> Item4;
    Item15 -.-> Item5;
    Item15 -.-> Item8;
    Item15 -.-> Item10;
    Item15 -.-> Item6;
    Item15 -.-> Item9;
    Item15 -.-> Item7;
    Item15 -.-> Item12;
    Item15 -.-> Item13;
    Item15 -.-> Item14;
    Item16 --> Item3;
    Item17 --> Item5;
    Item17 --> Item8;
    Item17 --> Item10;
    Item18 --> Item6;
    Item19 --> Item14;
```

# Condensed

```mermaid
graph TD
    Stmt16 --> Stmt15;
    Stmt4 --> Stmt3;
    Stmt6 --> Stmt2;
    Stmt6 --> Stmt3;
    Stmt6 -.-> Stmt4;
    Stmt7 --> Stmt3;
    Stmt7 --> Stmt6;
    Stmt8 --> Stmt3;
    Stmt8 --> Stmt6;
    Stmt8 -.-> Stmt4;
    Stmt8 -.-> Stmt7;
    Stmt17 --> Stmt7;
    Stmt17 --> Stmt16;
    Stmt17 -.-> Stmt12;
    Stmt17 -.-> Stmt3;
    Stmt17 -.-> Stmt6;
    Stmt17 -.-> Stmt8;
    Stmt17 -.-> Stmt4;
    Stmt17 -.-> Stmt1;
    Stmt18 --> Stmt7;
    Stmt18 -.-> Stmt17;
    Stmt13 --> Stmt12;
    Stmt13 --> Stmt3;
    Stmt13 --> Stmt6;
    Stmt13 --> Stmt8;
    Stmt12 --> Stmt1;
    Stmt12 --> Stmt3;
    Stmt12 --> Stmt6;
    Stmt12 --> Stmt8;
    Stmt10 -.-> Stmt4;
    Stmt10 -.-> Stmt7;
    Stmt10 -.-> Stmt8;
    Stmt19 --> Stmt17;
    Stmt19 -.-> Stmt13;
    Stmt19 -.-> Stmt1;
    Stmt19 -.-> Stmt3;
    Stmt19 -.-> Stmt6;
    Stmt19 -.-> Stmt8;
    Stmt19 -.-> Stmt4;
    Stmt19 -.-> Stmt7;
    Stmt19 -.-> Stmt2;
    Stmt19 -.-> Stmt18;
    Stmt19 -.-> Stmt12;
    Stmt19 -.-> Stmt10;
    Stmt14 --> Stmt13;
    Stmt9 --> Stmt3;
    Stmt9 --> Stmt6;
    Stmt9 --> Stmt8;
    Stmt5 --> Stmt4;
    Stmt11 --> Stmt10;
```
