# Items

Count: 7

## Item 1: Stmt 0, `VarDeclarator(0)`

```js
const dog = "dog";
```

- Declares: "`dog`"
- Write: "`dog`"

## Item 2: Stmt 1, `VarDeclarator(0)`

```js
const cat = "cat";
```

- Declares: "`cat`"
- Write: "`cat`"

## Item 3: Stmt 2, `VarDeclarator(0)`

```js
export const DOG = dog;
```

- Declares: "`DOG`"
- Reads: "`dog`"
- Write: "`DOG`"

## Item 4: Stmt 3, `VarDeclarator(0)`

```js
export const CHIMERA = cat + dog;
```

- Declares: "`CHIMERA`"
- Reads: "`cat`, `dog`"
- Write: "`CHIMERA`"

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
    Item3 --> Item1;
    Item4 --> Item2;
    Item4 --> Item1;
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
    Item3 --> Item1;
    Item4 --> Item2;
    Item4 --> Item1;
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
    Item3 --> Item1;
    Item4 --> Item2;
    Item4 --> Item1;
    Item5 -.-> Item1;
    Item5 -.-> Item3;
    Item5 -.-> Item4;
    Item5 -.-> Item2;
    Item6 --> Item3;
    Item7 --> Item4;
```

# Final

```mermaid
graph TD
    N0["Items: [ItemId(0, VarDeclarator(0)), ItemId(1, VarDeclarator(0)), ItemId(2, VarDeclarator(0)), ItemId(3, VarDeclarator(0)), ItemId(ModuleEvaluation)]"];
    N1["Items: [ItemId(Export((Atom('DOG' type=inline), #0)))]"];
    N2["Items: [ItemId(Export((Atom('CHIMERA' type=inline), #0)))]"];
    N1 --> N0;
    N2 --> N0;
```

# Modules (dev)

## Module 1

```js
"turbopack://chunk-0";
const dog = "dog";
const cat = "cat";
export const DOG = dog;
export const CHIMERA = cat + dog;
("module evaluation");
```

## Module 2

```js
"turbopack://chunk-1";
import "turbopack://chunk-0.js";
export { DOG };
```

## Module 3

```js
"turbopack://chunk-2";
import "turbopack://chunk-0.js";
export { CHIMERA };
```

# Modules (prod)

## Module 1

```js
"turbopack://chunk-0";
"module evaluation";
```

## Module 2

```js
"turbopack://chunk-1";
import "turbopack://chunk-2.js";
export const DOG = dog;
export { DOG };
```

## Module 3

```js
"turbopack://chunk-2";
const dog = "dog";
const cat = "cat";
export const CHIMERA = cat + dog;
export { CHIMERA };
```
