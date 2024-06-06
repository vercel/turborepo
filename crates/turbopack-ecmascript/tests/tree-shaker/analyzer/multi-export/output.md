# Items

Count: 6

## Item 1: Stmt 0, `VarDeclarator(0)`

```js
const dog = "dog";

```

- Declares: `dog`
- Write: `dog`

## Item 2: Stmt 1, `VarDeclarator(0)`

```js
const cat = "cat";

```

- Declares: `cat`
- Write: `cat`

## Item 3: Stmt 2, `Normal`

```js
export { dog as DOG, cat };

```

- Side effects
- Reads: `dog`, `cat`

# Phase 1
```mermaid
graph TD
    Item1;
    Item2;
    Item3;
    Item4;
    Item4["ModuleEvaluation"];
    Item5;
    Item5["export DOG"];
    Item6;
    Item6["export cat"];
```
# Phase 2
```mermaid
graph TD
    Item1;
    Item2;
    Item3;
    Item4;
    Item4["ModuleEvaluation"];
    Item5;
    Item5["export DOG"];
    Item6;
    Item6["export cat"];
    Item3 --> Item1;
    Item3 --> Item2;
```
# Phase 3
```mermaid
graph TD
    Item1;
    Item2;
    Item3;
    Item4;
    Item4["ModuleEvaluation"];
    Item5;
    Item5["export DOG"];
    Item6;
    Item6["export cat"];
    Item3 --> Item1;
    Item3 --> Item2;
```
# Phase 4
```mermaid
graph TD
    Item1;
    Item2;
    Item3;
    Item4;
    Item4["ModuleEvaluation"];
    Item5;
    Item5["export DOG"];
    Item6;
    Item6["export cat"];
    Item3 --> Item1;
    Item3 --> Item2;
    Item4 --> Item3;
    Item6 --> Item2;
```
# Final
```mermaid
graph TD
    N0["Items: [ItemId(ModuleEvaluation), ItemId(0, VarDeclarator(0)), ItemId(2, Normal)]"];
    N1["Items: [ItemId(Export((&quot;DOG&quot;, #1), &quot;DOG&quot;))]"];
    N2["Items: [ItemId(Export((&quot;cat&quot;, #2), &quot;cat&quot;))]"];
    N3["Items: [ItemId(1, VarDeclarator(0))]"];
    N0 --> N3;
    N2 --> N3;
```
# Entrypoints

```
{
    ModuleEvaluation: 0,
    Export(
        "cat",
    ): 2,
    Export(
        "DOG",
    ): 1,
}
```


# Modules (dev)
## Part 0
```js
import { cat } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 3
};
"module evaluation";
const dog = "dog";
export { dog as DOG, cat };
export { dog } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 1
```js
export { DOG as DOG };

```
## Part 2
```js
import { cat } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 3
};
export { cat as cat };

```
## Part 3
```js
const cat = "cat";
export { cat } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Merged (module eval)
```js
import { cat } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 3
};
"module evaluation";
const dog = "dog";
export { dog as DOG, cat };
export { dog } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
# Entrypoints

```
{
    ModuleEvaluation: 0,
    Export(
        "cat",
    ): 2,
    Export(
        "DOG",
    ): 1,
}
```


# Modules (prod)
## Part 0
```js
import { cat } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 3
};
"module evaluation";
const dog = "dog";
export { dog as DOG, cat };
export { dog } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 1
```js
export { DOG as DOG };

```
## Part 2
```js
import { cat } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 3
};
export { cat as cat };

```
## Part 3
```js
const cat = "cat";
export { cat } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Merged (module eval)
```js
import { cat } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 3
};
"module evaluation";
const dog = "dog";
export { dog as DOG, cat };
export { dog } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
