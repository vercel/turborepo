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
    Item3 -.-> Item2;
    Item5 --> Item1;
    Item6 --> Item1;
    Item6 --> Item3;
    Item6 -.-> Item2;
    Item6 -.-> Item5;
    Item8 --> Item1;
    Item9 --> Item1;
    Item9 --> Item6;
    Item9 -.-> Item2;
    Item9 -.-> Item3;
    Item9 -.-> Item5;
    Item9 -.-> Item8;
    Item10 --> Item1;
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
    Item3 -.-> Item2;
    Item5 --> Item1;
    Item6 --> Item1;
    Item6 --> Item3;
    Item6 -.-> Item2;
    Item6 -.-> Item5;
    Item8 --> Item1;
    Item9 --> Item1;
    Item9 --> Item6;
    Item9 -.-> Item2;
    Item9 -.-> Item3;
    Item9 -.-> Item5;
    Item9 -.-> Item8;
    Item10 --> Item1;
    Item10 --> Item4;
    Item10 --> Item7;
    Item12 --> Item11;
    Item4 --> Item1;
    Item7 -.-> Item2;
    Item7 -.-> Item3;
    Item7 -.-> Item5;
    Item7 -.-> Item6;
    Item7 -.-> Item8;
    Item7 -.-> Item9;
    Item7 -.-> Item10;
    Item13 --> Item11;
    Item13 --> Item1;
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
    Item3 -.-> Item2;
    Item5 --> Item1;
    Item6 --> Item1;
    Item6 --> Item3;
    Item6 -.-> Item2;
    Item6 -.-> Item5;
    Item8 --> Item1;
    Item9 --> Item1;
    Item9 --> Item6;
    Item9 -.-> Item2;
    Item9 -.-> Item3;
    Item9 -.-> Item5;
    Item9 -.-> Item8;
    Item10 --> Item1;
    Item10 --> Item4;
    Item10 --> Item7;
    Item12 --> Item11;
    Item4 --> Item1;
    Item7 -.-> Item2;
    Item7 -.-> Item3;
    Item7 -.-> Item5;
    Item7 -.-> Item6;
    Item7 -.-> Item8;
    Item7 -.-> Item9;
    Item7 -.-> Item10;
    Item13 --> Item11;
    Item13 --> Item1;
    Item14 --> Item9;
    Item14 -.-> Item1;
    Item14 -.-> Item2;
    Item14 -.-> Item3;
    Item14 -.-> Item5;
    Item14 -.-> Item6;
    Item14 -.-> Item8;
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
