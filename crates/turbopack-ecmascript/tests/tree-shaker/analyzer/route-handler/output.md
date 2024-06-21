# Items

Count: 9

## Item 1: Stmt 0, `ImportOfModule`

```js
import { NextRequest, NextResponse } from 'next/server';

```

- Hoisted
- Side effects

## Item 2: Stmt 0, `ImportBinding(0)`

```js
import { NextRequest, NextResponse } from 'next/server';

```

- Hoisted
- Declares: `NextRequest`

## Item 3: Stmt 0, `ImportBinding(1)`

```js
import { NextRequest, NextResponse } from 'next/server';

```

- Hoisted
- Declares: `NextResponse`

## Item 4: Stmt 1, `VarDeclarator(0)`

```js
export const runtime = 'edge';

```

- Declares: `runtime`
- Write: `runtime`

## Item 5: Stmt 2, `VarDeclarator(0)`

```js
let count = 0;

```

- Declares: `count`
- Write: `count`

## Item 6: Stmt 3, `VarDeclarator(0)`

```js
export const GET = async (req)=>{
    await fetch(req.nextUrl);
    count++;
    return NextResponse.json({
        count
    });
};

```

- Side effects
- Declares: `GET`
- Reads: `count`, `NextResponse`
- Write: `GET`, `NextResponse`

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
    Item7["ModuleEvaluation"];
    Item8;
    Item8["export runtime"];
    Item9;
    Item9["export GET"];
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
    Item7["ModuleEvaluation"];
    Item8;
    Item8["export runtime"];
    Item9;
    Item9["export GET"];
    Item6 --> Item5;
    Item6 --> Item3;
    Item6 --> Item1;
    Item8 --> Item4;
    Item9 --> Item6;
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
    Item7["ModuleEvaluation"];
    Item8;
    Item8["export runtime"];
    Item9;
    Item9["export GET"];
    Item6 --> Item5;
    Item6 --> Item3;
    Item6 --> Item1;
    Item8 --> Item4;
    Item9 --> Item6;
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
    Item7["ModuleEvaluation"];
    Item8;
    Item8["export runtime"];
    Item9;
    Item9["export GET"];
    Item6 --> Item5;
    Item6 --> Item3;
    Item6 --> Item1;
    Item8 --> Item4;
    Item9 --> Item6;
    Item7 --> Item1;
    Item7 --> Item6;
```
# Final
```mermaid
graph TD
    N0["Items: [ItemId(ModuleEvaluation)]"];
    N1["Items: [ItemId(Export((&quot;runtime&quot;, #2), &quot;runtime&quot;)), ItemId(1, VarDeclarator(0))]"];
    N2["Items: [ItemId(Export((&quot;GET&quot;, #2), &quot;GET&quot;))]"];
    N3["Items: [ItemId(0, ImportOfModule)]"];
    N4["Items: [ItemId(0, ImportBinding(1)), ItemId(2, VarDeclarator(0)), ItemId(3, VarDeclarator(0))]"];
    N0 --> N3;
    N0 --> N4;
    N2 --> N4;
    N4 --> N3;
```
# Entrypoints

```
{
    ModuleEvaluation: 0,
    Export(
        "GET",
    ): 2,
    Export(
        "runtime",
    ): 1,
}
```


# Modules (dev)
## Part 0
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 3
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 4
};
"module evaluation";

```
## Part 1
```js
export { runtime };
const runtime = 'edge';
export { runtime } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 2
```js
import { GET } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 4
};
export { GET };

```
## Part 3
```js
import 'next/server';

```
## Part 4
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 3
};
import { NextResponse } from 'next/server';
let count = 0;
const GET = async (req)=>{
    await fetch(req.nextUrl);
    count++;
    return NextResponse.json({
        count
    });
};
export { NextResponse } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { count } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { GET } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Merged (module eval)
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 3
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 4
};
"module evaluation";

```
# Entrypoints

```
{
    ModuleEvaluation: 0,
    Export(
        "GET",
    ): 2,
    Export(
        "runtime",
    ): 1,
}
```


# Modules (prod)
## Part 0
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 3
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 4
};
"module evaluation";

```
## Part 1
```js
export { runtime };
const runtime = 'edge';
export { runtime } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 2
```js
import { GET } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 4
};
export { GET };

```
## Part 3
```js
import 'next/server';

```
## Part 4
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 3
};
import { NextResponse } from 'next/server';
let count = 0;
const GET = async (req)=>{
    await fetch(req.nextUrl);
    count++;
    return NextResponse.json({
        count
    });
};
export { NextResponse } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { count } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { GET } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Merged (module eval)
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 3
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 4
};
"module evaluation";

```
