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
    N0["Items: [ItemId(0, VarDeclarator(0))]"];
    N1["Items: [ItemId(1, Normal)]"];
    N2["Items: [ItemId(2, Normal)]"];
    N3["Items: [ItemId(3, Normal)]"];
    N4["Items: [ItemId(4, Normal)]"];
    N5["Items: [ItemId(5, Normal)]"];
    N6["Items: [ItemId(6, Normal)]"];
    N7["Items: [ItemId(7, Normal)]"];
    N8["Items: [ItemId(8, Normal)]"];
    N9["Items: [ItemId(9, VarDeclarator(0))]"];
    N10["Items: [ItemId(10, VarDeclarator(0))]"];
    N11["Items: [ItemId(11, VarDeclarator(0))]"];
    N12["Items: [ItemId(12, Normal)]"];
    N13["Items: [ItemId(ModuleEvaluation), ItemId(8, Normal), ItemId(0, VarDeclarator(0)), ItemId(1, Normal), ItemId(4, Normal), ItemId(7, Normal), ItemId(2, Normal), ItemId(5, Normal), ItemId(6, Normal), ItemId(3, Normal)]"];
    N14["Items: [ItemId(Export((Atom('dogRef' type=inline), #0))), ItemId(9, VarDeclarator(0))]"];
    N15["Items: [ItemId(Export((Atom('cat' type=inline), #0))), ItemId(10, VarDeclarator(0))]"];
    N16["Items: [ItemId(Export((Atom('initialCat' type=dynamic), #0))), ItemId(11, VarDeclarator(0))]"];
    N17["Items: [ItemId(Export((Atom('getChimera' type=dynamic), #0))), ItemId(12, Normal)]"];
    N1 --> N13;
    N2 --> N13;
    N3 --> N13;
    N4 --> N13;
    N5 --> N13;
    N6 --> N13;
    N6 --> N14;
    N7 --> N13;
    N8 --> N13;
    N9 --> N13;
    N11 --> N15;
    N12 --> N15;
    N12 --> N13;
    N13 --> N13;
    N13 --> N14;
    N13 --> N15;
    N13 --> N16;
    N13 --> N17;
    N14 --> N14;
    N14 --> N13;
    N15 --> N15;
    N16 --> N16;
    N16 --> N15;
    N17 --> N17;
    N17 --> N15;
    N17 --> N13;
```
