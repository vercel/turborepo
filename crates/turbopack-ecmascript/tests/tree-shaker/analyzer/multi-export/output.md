# Items

Count: 5

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

# Phase 1
```mermaid
graph TD
    Item1;
    Item1["ModuleEvaluation"];
    Item2;
    Item2["export cat"];
    Item3;
    Item3["export DOG"];
    Item4;
    Item5;
```
# Phase 2
```mermaid
graph TD
    Item1;
    Item1["ModuleEvaluation"];
    Item2;
    Item2["export cat"];
    Item3;
    Item3["export DOG"];
    Item4;
    Item5;
    Item5 -.-> Item2;
```
# Phase 3
```mermaid
graph TD
    Item1;
    Item1["ModuleEvaluation"];
    Item2;
    Item2["export cat"];
    Item3;
    Item3["export DOG"];
    Item4;
    Item5;
    Item5 -.-> Item2;
```
# Phase 4
```mermaid
graph TD
    Item1;
    Item1["ModuleEvaluation"];
    Item2;
    Item2["export cat"];
    Item3;
    Item3["export DOG"];
    Item4;
    Item5;
    Item5 -.-> Item2;
    Item2 --> Item5;
    Item3 --> Item4;
```
# Final
```mermaid
graph TD
    N0["Items: [ItemId(ModuleEvaluation)]"];
    N1["Items: [ItemId(Export((&quot;cat&quot;, #2), &quot;cat&quot;)), ItemId(1, VarDeclarator(0))]"];
    N2["Items: [ItemId(Export((&quot;dog&quot;, #2), &quot;DOG&quot;)), ItemId(0, VarDeclarator(0))]"];
```
# Entrypoints

```
{
    ModuleEvaluation: 0,
    Export(
        "cat",
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
export { cat as cat };
const cat = "cat";
export { cat } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 2
```js
export { dog as DOG };
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
        "cat",
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
export { cat as cat };
const cat = "cat";
export { cat } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 2
```js
export { dog as DOG };
const dog = "dog";
export { dog } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Merged (module eval)
```js
"module evaluation";

```
