# Items

Count: 7

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

## Item 3: Stmt 2, `VarDeclarator(0)`

```js
export const DOG = dog;

```

- Declares: `DOG`
- Reads: `dog`
- Write: `DOG`

## Item 4: Stmt 3, `VarDeclarator(0)`

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
    Item2;
    Item3;
    Item4;
    Item5;
    Item5["ModuleEvaluation"];
    Item6;
    Item6["export DOG"];
    Item7;
    Item7["export CHIMERA"];
```
# Phase 2
```mermaid
graph TD
    Item1;
    Item2;
    Item3;
    Item4;
    Item5;
    Item5["ModuleEvaluation"];
    Item6;
    Item6["export DOG"];
    Item7;
    Item7["export CHIMERA"];
    Item1 -.-> Item1;
    Item2 -.-> Item2;
    Item3 -.-> Item3;
    Item4 -.-> Item4;
```
# Phase 3
```mermaid
graph TD
    Item1;
    Item2;
    Item3;
    Item4;
    Item5;
    Item5["ModuleEvaluation"];
    Item6;
    Item6["export DOG"];
    Item7;
    Item7["export CHIMERA"];
    Item1 -.-> Item1;
    Item2 -.-> Item2;
    Item3 -.-> Item3;
    Item4 -.-> Item4;
```
# Phase 4
```mermaid
graph TD
    Item1;
    Item2;
    Item3;
    Item4;
    Item5;
    Item5["ModuleEvaluation"];
    Item6;
    Item6["export DOG"];
    Item7;
    Item7["export CHIMERA"];
    Item1 -.-> Item1;
    Item2 -.-> Item2;
    Item3 -.-> Item3;
    Item4 -.-> Item4;
```
# Final
```mermaid
graph TD
    N0["Items: [ItemId(ModuleEvaluation)]"];
    N1["Items: [ItemId(Export((&quot;DOG&quot;, #2), &quot;DOG&quot;))]"];
    N2["Items: [ItemId(Export((&quot;CHIMERA&quot;, #2), &quot;CHIMERA&quot;))]"];
```
# Entrypoints

```
{
    ModuleEvaluation: 0,
    Export(
        "DOG",
    ): 1,
    Export(
        "CHIMERA",
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
export { DOG };

```
## Part 2
```js
export { CHIMERA };

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
        "DOG",
    ): 1,
    Export(
        "CHIMERA",
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
export { DOG };

```
## Part 2
```js
export { CHIMERA };

```
## Merged (module eval)
```js
"module evaluation";

```
