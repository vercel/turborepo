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
    N0["Statements: [1]"];
    N1["Statements: [0]"];
    N2["Statements: [3]"];
    N3["Statements: [6]"];
    N4["Statements: [2]"];
    N5["Statements: [5]"];
    N6["Statements: [4]"];
    N4 --> N1;
    N2 --> N0;
    N2 --> N1;
    N6 -.-> N1;
    N6 -.-> N4;
    N6 -.-> N2;
    N6 -.-> N0;
    N5 --> N4;
    N3 --> N2;
```
