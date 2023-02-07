# Items

Count: 18

## Item 1: Stmt 0, `VarDeclarator(0)`

```js
let dog = "dog";
```

- Declares: "`dog`"
- Write: "`dog`"

## Item 2: Stmt 1, `Normal`

```js
dog += "!";
```

- Reads: "`dog`"
- Write: "`dog`"

## Item 3: Stmt 2, `Normal`

```js
console.log(dog);
```

- Side effects
- Reads: "`console`, `dog`"

## Item 4: Stmt 3, `Normal`

```js
function getDog() {
  return dog;
}
```

- Hoisted
- Declares: "`getDog`"
- Reads (eventual): "`dog`"

## Item 5: Stmt 4, `Normal`

```js
dog += "!";
```

- Reads: "`dog`"
- Write: "`dog`"

## Item 6: Stmt 5, `Normal`

```js
console.log(dog);
```

- Side effects
- Reads: "`console`, `dog`"

## Item 7: Stmt 6, `Normal`

```js
function setDog(newDog) {
  dog = newDog;
}
```

- Hoisted
- Declares: "`setDog`"
- Reads (eventual): "`newDog`"
- Write (eventual): "`newDog`, `dog`"

## Item 8: Stmt 7, `Normal`

```js
dog += "!";
```

- Reads: "`dog`"
- Write: "`dog`"

## Item 9: Stmt 8, `Normal`

```js
console.log(dog);
```

- Side effects
- Reads: "`console`, `dog`"

## Item 10: Stmt 9, `VarDeclarator(0)`

```js
export const dogRef = {
  initial: dog,
  get: getDog,
  set: setDog,
};
```

- Declares: "`dogRef`"
- Reads: "`dog`, `getDog`, `setDog`"
- Write: "`dogRef`"

## Item 11: Stmt 10, `VarDeclarator(0)`

```js
export let cat = "cat";
```

- Declares: "`cat`"
- Write: "`cat`"

## Item 12: Stmt 11, `VarDeclarator(0)`

```js
export const initialCat = cat;
```

- Declares: "`initialCat`"
- Reads: "`cat`"
- Write: "`initialCat`"

## Item 13: Stmt 12, `Normal`

```js
export function getChimera() {
  return cat + dog;
}
```

- Hoisted
- Declares: "`getChimera`"
- Reads (eventual): "`cat`, `dog`"

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
    Item15["export dogRef"];
    Item16;
    Item16["export cat"];
    Item17;
    Item17["export initialCat"];
    Item18;
    Item18["export getChimera"];
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
    Item15["export dogRef"];
    Item16;
    Item16["export cat"];
    Item17;
    Item17["export initialCat"];
    Item18;
    Item18["export getChimera"];
    Item2 --> Item1;
    Item3 --> Item1;
    Item3 --> Item2;
    Item5 --> Item1;
    Item5 --> Item2;
    Item5 -.-> Item3;
    Item6 --> Item1;
    Item6 --> Item2;
    Item6 --> Item5;
    Item6 --> Item3;
    Item8 --> Item1;
    Item8 --> Item2;
    Item8 --> Item5;
    Item8 -.-> Item3;
    Item8 -.-> Item6;
    Item9 --> Item1;
    Item9 --> Item2;
    Item9 --> Item5;
    Item9 --> Item8;
    Item9 --> Item6;
    Item9 -.-> Item3;
    Item10 --> Item1;
    Item10 --> Item2;
    Item10 --> Item5;
    Item10 --> Item8;
    Item10 --> Item4;
    Item10 --> Item7;
    Item12 --> Item11;
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
    Item15["export dogRef"];
    Item16;
    Item16["export cat"];
    Item17;
    Item17["export initialCat"];
    Item18;
    Item18["export getChimera"];
    Item2 --> Item1;
    Item3 --> Item1;
    Item3 --> Item2;
    Item5 --> Item1;
    Item5 --> Item2;
    Item5 -.-> Item3;
    Item6 --> Item1;
    Item6 --> Item2;
    Item6 --> Item5;
    Item6 --> Item3;
    Item8 --> Item1;
    Item8 --> Item2;
    Item8 --> Item5;
    Item8 -.-> Item3;
    Item8 -.-> Item6;
    Item9 --> Item1;
    Item9 --> Item2;
    Item9 --> Item5;
    Item9 --> Item8;
    Item9 --> Item6;
    Item9 -.-> Item3;
    Item10 --> Item1;
    Item10 --> Item2;
    Item10 --> Item5;
    Item10 --> Item8;
    Item10 --> Item4;
    Item10 --> Item7;
    Item12 --> Item11;
    Item4 --> Item1;
    Item4 --> Item2;
    Item4 --> Item5;
    Item4 --> Item8;
    Item7 -.-> Item3;
    Item7 -.-> Item6;
    Item7 -.-> Item9;
    Item7 -.-> Item10;
    Item13 --> Item11;
    Item13 --> Item1;
    Item13 --> Item2;
    Item13 --> Item5;
    Item13 --> Item8;
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
    Item15["export dogRef"];
    Item16;
    Item16["export cat"];
    Item17;
    Item17["export initialCat"];
    Item18;
    Item18["export getChimera"];
    Item2 --> Item1;
    Item3 --> Item1;
    Item3 --> Item2;
    Item5 --> Item1;
    Item5 --> Item2;
    Item5 -.-> Item3;
    Item6 --> Item1;
    Item6 --> Item2;
    Item6 --> Item5;
    Item6 --> Item3;
    Item8 --> Item1;
    Item8 --> Item2;
    Item8 --> Item5;
    Item8 -.-> Item3;
    Item8 -.-> Item6;
    Item9 --> Item1;
    Item9 --> Item2;
    Item9 --> Item5;
    Item9 --> Item8;
    Item9 --> Item6;
    Item9 -.-> Item3;
    Item10 --> Item1;
    Item10 --> Item2;
    Item10 --> Item5;
    Item10 --> Item8;
    Item10 --> Item4;
    Item10 --> Item7;
    Item12 --> Item11;
    Item4 --> Item1;
    Item4 --> Item2;
    Item4 --> Item5;
    Item4 --> Item8;
    Item7 -.-> Item3;
    Item7 -.-> Item6;
    Item7 -.-> Item9;
    Item7 -.-> Item10;
    Item13 --> Item11;
    Item13 --> Item1;
    Item13 --> Item2;
    Item13 --> Item5;
    Item13 --> Item8;
    Item14 --> Item9;
    Item14 -.-> Item1;
    Item14 -.-> Item2;
    Item14 -.-> Item5;
    Item14 -.-> Item8;
    Item14 -.-> Item3;
    Item14 -.-> Item6;
    Item14 -.-> Item10;
    Item14 -.-> Item7;
    Item14 -.-> Item11;
    Item14 -.-> Item12;
    Item14 -.-> Item4;
    Item14 -.-> Item13;
    Item15 --> Item10;
    Item16 --> Item11;
    Item17 --> Item12;
    Item18 --> Item13;
```

# Final

```mermaid
graph TD
    N0["Statements: [10]"];
    N1["Statements: [15]"];
    N2["Statements: [11]"];
    N3["Statements: [16]"];
    N4["Statements: [0]"];
    N5["Statements: [1]"];
    N6["Statements: [2]"];
    N7["Statements: [4]"];
    N8["Statements: [5]"];
    N9["Statements: [7]"];
    N10["Statements: [12]"];
    N11["Statements: [17]"];
    N12["Statements: [3]"];
    N13["Statements: [8]"];
    N14["Statements: [9, 6]"];
    N15["Statements: [14]"];
    N16["Statements: [13]"];
    N5 --> N4;
    N6 --> N4;
    N6 --> N5;
    N7 --> N4;
    N7 --> N5;
    N7 -.-> N6;
    N8 --> N4;
    N8 --> N5;
    N8 --> N7;
    N8 --> N6;
    N9 --> N4;
    N9 --> N5;
    N9 --> N7;
    N9 -.-> N6;
    N9 -.-> N8;
    N13 --> N4;
    N13 --> N5;
    N13 --> N7;
    N13 --> N9;
    N13 --> N8;
    N13 -.-> N6;
    N14 --> N4;
    N14 --> N5;
    N14 --> N7;
    N14 --> N9;
    N14 --> N12;
    N2 --> N0;
    N12 --> N4;
    N12 --> N5;
    N12 --> N7;
    N12 --> N9;
    N14 -.-> N6;
    N14 -.-> N8;
    N14 -.-> N13;
    N10 --> N0;
    N10 --> N4;
    N10 --> N5;
    N10 --> N7;
    N10 --> N9;
    N16 --> N13;
    N16 -.-> N4;
    N16 -.-> N5;
    N16 -.-> N7;
    N16 -.-> N9;
    N16 -.-> N6;
    N16 -.-> N8;
    N16 -.-> N14;
    N16 -.-> N0;
    N16 -.-> N2;
    N16 -.-> N12;
    N16 -.-> N10;
    N15 --> N14;
    N1 --> N0;
    N3 --> N2;
    N11 --> N10;
```
