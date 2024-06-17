# Items

Count: 7

## Item 4: Stmt 0, `VarDeclarator(0)`

```js
const dog = "dog";

```

- Declares: `dog`
- Write: `dog`

## Item 5: Stmt 1, `VarDeclarator(0)`

```js
const cat = "cat";

```

- Declares: `cat`
- Write: `cat`

## Item 6: Stmt 2, `VarDeclarator(0)`

```js
export const DOG = dog;

```

- Declares: `DOG`
- Reads: `dog`
- Write: `DOG`

## Item 7: Stmt 3, `VarDeclarator(0)`

```js
export const CHIMERA = cat + dog;

```

- Declares: `CHIMERA`
- Reads: `cat`, `dog`
- Write: `CHIMERA`

# Phase 1
```mermaid
graph TD
    Item1;
    Item1["ModuleEvaluation"];
    Item2;
    Item2["export CHIMERA"];
    Item3;
    Item3["export DOG"];
    Item4;
    Item5;
    Item6;
    Item7;
```
# Phase 2
```mermaid
graph TD
    Item1;
    Item1["ModuleEvaluation"];
    Item2;
    Item2["export CHIMERA"];
    Item3;
    Item3["export DOG"];
    Item4;
    Item5;
    Item6;
    Item7;
    Item6 --> Item4;
    Item6 -.-> Item3;
    Item7 --> Item5;
    Item7 --> Item4;
    Item7 -.-> Item2;
```
# Phase 3
```mermaid
graph TD
    Item1;
    Item1["ModuleEvaluation"];
    Item2;
    Item2["export CHIMERA"];
    Item3;
    Item3["export DOG"];
    Item4;
    Item5;
    Item6;
    Item7;
    Item6 --> Item4;
    Item6 -.-> Item3;
    Item7 --> Item5;
    Item7 --> Item4;
    Item7 -.-> Item2;
```
# Phase 4
```mermaid
graph TD
    Item1;
    Item1["ModuleEvaluation"];
    Item2;
    Item2["export CHIMERA"];
    Item3;
    Item3["export DOG"];
    Item4;
    Item5;
    Item6;
    Item7;
    Item6 --> Item4;
    Item6 -.-> Item3;
    Item7 --> Item5;
    Item7 --> Item4;
    Item7 -.-> Item2;
    Item2 --> Item7;
    Item3 --> Item6;
```
# Final
```mermaid
graph TD
    N0["Items: [ItemId(ModuleEvaluation)]"];
    N1["Items: [ItemId(Export((&quot;CHIMERA&quot;, #2), &quot;CHIMERA&quot;)), ItemId(1, VarDeclarator(0)), ItemId(3, VarDeclarator(0))]"];
    N2["Items: [ItemId(Export((&quot;DOG&quot;, #2), &quot;DOG&quot;)), ItemId(2, VarDeclarator(0))]"];
    N3["Items: [ItemId(0, VarDeclarator(0))]"];
    N1 --> N3;
    N2 --> N3;
```
# Entrypoints

```
{
    ModuleEvaluation: 0,
    Export(
        "CHIMERA",
    ): 1,
    Export(
        "DOG",
    ): 2,
}
```


# Modules (dev)
## Part 0
```js
"module evaluation";

```
## Part 1
```js
import { dog } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 3
};
export { CHIMERA };
const cat = "cat";
const CHIMERA = cat + dog;
export { cat } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { CHIMERA } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 2
```js
import { dog } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 3
};
export { DOG };
const DOG = dog;
export { DOG } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 3
```js
const dog = "dog";
export { dog } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Merged (module eval)
```js
"module evaluation";

```
# Entrypoints

```
{
    ModuleEvaluation: 0,
    Export(
        "CHIMERA",
    ): 1,
    Export(
        "DOG",
    ): 2,
}
```


# Modules (prod)
## Part 0
```js
"module evaluation";

```
## Part 1
```js
import { dog } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 3
};
export { CHIMERA };
const cat = "cat";
const CHIMERA = cat + dog;
export { cat } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { CHIMERA } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 2
```js
import { dog } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 3
};
export { DOG };
const DOG = dog;
export { DOG } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 3
```js
const dog = "dog";
export { dog } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Merged (module eval)
```js
"module evaluation";

```
