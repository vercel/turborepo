# Items

Count: 7

## Item 1: Stmt 0, `VarDeclarator(0)`

```js
const dog = "dog";
```

- Side effects
- Declares: "`dog`"
- Write: "`dog`"

## Item 2: Stmt 1, `VarDeclarator(0)`

```js
const cat = "cat";
```

- Side effects
- Declares: "`cat`"
- Write: "`cat`"

## Item 3: Stmt 2, `VarDeclarator(0)`

```js
export const DOG = dog;
```

- Side effects
- Declares: "`DOG`"
- Reads: "`dog`"
- Write: "`DOG`"

## Item 4: Stmt 3, `VarDeclarator(0)`

```js
export const CHIMERA = cat + dog;
```

- Side effects
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
    Item2 --> Item1;
    Item3 --> Item1;
    Item3 --> Item2;
    Item4 --> Item2;
    Item4 --> Item1;
    Item4 --> Item3;
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
    Item2 --> Item1;
    Item3 --> Item1;
    Item3 --> Item2;
    Item4 --> Item2;
    Item4 --> Item1;
    Item4 --> Item3;
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
    Item2 --> Item1;
    Item3 --> Item1;
    Item3 --> Item2;
    Item4 --> Item2;
    Item4 --> Item1;
    Item4 --> Item3;
    Item5 --> Item4;
    Item5 -.-> Item1;
    Item5 -.-> Item3;
    Item5 -.-> Item2;
    Item6 --> Item3;
    Item7 --> Item4;
```
