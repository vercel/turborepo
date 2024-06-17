# Items

Count: 18

## Item 6: Stmt 0, `VarDeclarator(0)`

```js
let dog = "dog";

```

- Declares: `dog`
- Write: `dog`

## Item 7: Stmt 1, `Normal`

```js
dog += "!";

```

- Reads: `dog`
- Write: `dog`

## Item 8: Stmt 2, `Normal`

```js
console.log(dog);

```

- Side effects
- Reads: `dog`

## Item 9: Stmt 3, `Normal`

```js
function getDog() {
    return dog;
}

```

- Hoisted
- Declares: `getDog`
- Reads (eventual): `dog`
- Write: `getDog`

## Item 10: Stmt 4, `Normal`

```js
dog += "!";

```

- Reads: `dog`
- Write: `dog`

## Item 11: Stmt 5, `Normal`

```js
console.log(dog);

```

- Side effects
- Reads: `dog`

## Item 12: Stmt 6, `Normal`

```js
function setDog(newDog) {
    dog = newDog;
}

```

- Hoisted
- Declares: `setDog`
- Write: `setDog`
- Write (eventual): `dog`

## Item 13: Stmt 7, `Normal`

```js
dog += "!";

```

- Reads: `dog`
- Write: `dog`

## Item 14: Stmt 8, `Normal`

```js
console.log(dog);

```

- Side effects
- Reads: `dog`

## Item 15: Stmt 9, `VarDeclarator(0)`

```js
export const dogRef = {
    initial: dog,
    get: getDog,
    set: setDog
};

```

- Declares: `dogRef`
- Reads: `dog`, `getDog`, `setDog`
- Write: `dogRef`

## Item 16: Stmt 10, `VarDeclarator(0)`

```js
export let cat = "cat";

```

- Declares: `cat`
- Write: `cat`

## Item 17: Stmt 11, `VarDeclarator(0)`

```js
export const initialCat = cat;

```

- Declares: `initialCat`
- Reads: `cat`
- Write: `initialCat`

## Item 18: Stmt 12, `Normal`

```js
export function getChimera() {
    return cat + dog;
}

```

- Hoisted
- Declares: `getChimera`
- Reads (eventual): `cat`, `dog`
- Write: `getChimera`

# Phase 1
```mermaid
graph TD
    Item1;
    Item1["ModuleEvaluation"];
    Item2;
    Item2["export cat"];
    Item3;
    Item3["export dogRef"];
    Item4;
    Item4["export getChimera"];
    Item5;
    Item5["export initialCat"];
    Item6;
    Item7;
    Item8;
    Item9;
    Item10;
    Item11;
    Item12;
    Item13;
    Item14;
    Item15;
    Item16;
    Item17;
    Item18;
```
# Phase 2
```mermaid
graph TD
    Item1;
    Item1["ModuleEvaluation"];
    Item2;
    Item2["export cat"];
    Item3;
    Item3["export dogRef"];
    Item4;
    Item4["export getChimera"];
    Item5;
    Item5["export initialCat"];
    Item6;
    Item7;
    Item8;
    Item9;
    Item10;
    Item11;
    Item12;
    Item13;
    Item14;
    Item15;
    Item16;
    Item17;
    Item18;
    Item4 --> Item18;
    Item7 --> Item6;
    Item8 --> Item7;
    Item8 --> Item6;
    Item8 -.-> Item2;
    Item10 --> Item7;
    Item10 --> Item6;
    Item10 -.-> Item8;
    Item11 --> Item10;
    Item11 --> Item6;
    Item11 --> Item8;
    Item11 -.-> Item2;
    Item13 --> Item10;
    Item13 --> Item6;
    Item13 -.-> Item11;
    Item14 --> Item13;
    Item14 --> Item6;
    Item14 --> Item8;
    Item14 --> Item11;
    Item14 -.-> Item2;
    Item15 --> Item13;
    Item15 --> Item6;
    Item15 --> Item9;
    Item15 --> Item12;
    Item15 -.-> Item3;
    Item16 -.-> Item2;
    Item17 --> Item16;
    Item17 -.-> Item5;
```
# Phase 3
```mermaid
graph TD
    Item1;
    Item1["ModuleEvaluation"];
    Item2;
    Item2["export cat"];
    Item3;
    Item3["export dogRef"];
    Item4;
    Item4["export getChimera"];
    Item5;
    Item5["export initialCat"];
    Item6;
    Item7;
    Item8;
    Item9;
    Item10;
    Item11;
    Item12;
    Item13;
    Item14;
    Item15;
    Item16;
    Item17;
    Item18;
    Item4 --> Item18;
    Item7 --> Item6;
    Item8 --> Item7;
    Item8 --> Item6;
    Item8 -.-> Item2;
    Item10 --> Item7;
    Item10 --> Item6;
    Item10 -.-> Item8;
    Item11 --> Item10;
    Item11 --> Item6;
    Item11 --> Item8;
    Item11 -.-> Item2;
    Item13 --> Item10;
    Item13 --> Item6;
    Item13 -.-> Item11;
    Item14 --> Item13;
    Item14 --> Item6;
    Item14 --> Item8;
    Item14 --> Item11;
    Item14 -.-> Item2;
    Item15 --> Item13;
    Item15 --> Item6;
    Item15 --> Item9;
    Item15 --> Item12;
    Item15 -.-> Item3;
    Item16 -.-> Item2;
    Item17 --> Item16;
    Item17 -.-> Item5;
    Item9 --> Item13;
    Item12 -.-> Item14;
    Item12 -.-> Item15;
    Item18 --> Item16;
    Item18 --> Item13;
```
# Phase 4
```mermaid
graph TD
    Item1;
    Item1["ModuleEvaluation"];
    Item2;
    Item2["export cat"];
    Item3;
    Item3["export dogRef"];
    Item4;
    Item4["export getChimera"];
    Item5;
    Item5["export initialCat"];
    Item6;
    Item7;
    Item8;
    Item9;
    Item10;
    Item11;
    Item12;
    Item13;
    Item14;
    Item15;
    Item16;
    Item17;
    Item18;
    Item4 --> Item18;
    Item7 --> Item6;
    Item8 --> Item7;
    Item8 --> Item6;
    Item8 -.-> Item2;
    Item10 --> Item7;
    Item10 --> Item6;
    Item10 -.-> Item8;
    Item11 --> Item10;
    Item11 --> Item6;
    Item11 --> Item8;
    Item11 -.-> Item2;
    Item13 --> Item10;
    Item13 --> Item6;
    Item13 -.-> Item11;
    Item14 --> Item13;
    Item14 --> Item6;
    Item14 --> Item8;
    Item14 --> Item11;
    Item14 -.-> Item2;
    Item15 --> Item13;
    Item15 --> Item6;
    Item15 --> Item9;
    Item15 --> Item12;
    Item15 -.-> Item3;
    Item16 -.-> Item2;
    Item17 --> Item16;
    Item17 -.-> Item5;
    Item9 --> Item13;
    Item12 -.-> Item14;
    Item12 -.-> Item15;
    Item18 --> Item16;
    Item18 --> Item13;
    Item1 --> Item8;
    Item1 --> Item11;
    Item1 --> Item14;
    Item2 --> Item16;
    Item3 --> Item15;
    Item5 --> Item17;
```
# Final
```mermaid
graph TD
    N0["Items: [ItemId(ModuleEvaluation)]"];
    N1["Items: [ItemId(Export((&quot;cat&quot;, #2), &quot;cat&quot;))]"];
    N2["Items: [ItemId(Export((&quot;dogRef&quot;, #2), &quot;dogRef&quot;)), ItemId(3, Normal), ItemId(6, Normal), ItemId(9, VarDeclarator(0))]"];
    N3["Items: [ItemId(Export((&quot;getChimera&quot;, #2), &quot;getChimera&quot;)), ItemId(12, Normal)]"];
    N4["Items: [ItemId(Export((&quot;initialCat&quot;, #2), &quot;initialCat&quot;)), ItemId(11, VarDeclarator(0))]"];
    N5["Items: [ItemId(0, VarDeclarator(0))]"];
    N6["Items: [ItemId(1, Normal)]"];
    N7["Items: [ItemId(2, Normal)]"];
    N8["Items: [ItemId(4, Normal)]"];
    N9["Items: [ItemId(5, Normal)]"];
    N10["Items: [ItemId(7, Normal)]"];
    N11["Items: [ItemId(8, Normal)]"];
    N12["Items: [ItemId(10, VarDeclarator(0))]"];
    N0 --> N7;
    N0 --> N9;
    N0 --> N11;
    N1 --> N12;
    N2 --> N10;
    N2 --> N11;
    N2 --> N5;
    N3 --> N12;
    N3 --> N10;
    N4 --> N12;
    N6 --> N5;
    N7 --> N6;
    N7 --> N5;
    N7 --> N1;
    N8 --> N6;
    N8 --> N5;
    N8 --> N7;
    N9 --> N8;
    N9 --> N5;
    N9 --> N7;
    N9 --> N1;
    N10 --> N8;
    N10 --> N5;
    N10 --> N9;
    N11 --> N10;
    N11 --> N5;
    N11 --> N7;
    N11 --> N9;
    N11 --> N1;
    N12 --> N1;
```
# Entrypoints

```
{
    ModuleEvaluation: 0,
    Export(
        "getChimera",
    ): 3,
    Export(
        "initialCat",
    ): 4,
    Export(
        "cat",
    ): 1,
    Export(
        "dogRef",
    ): 2,
}
```


# Modules (dev)
## Part 0
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 7
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 9
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 11
};
"module evaluation";

```
## Part 1
```js
import { cat } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 12
};
export { cat };

```
## Part 2
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 10
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 11
};
import { dog } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
export { dogRef };
function getDog() {
    return dog;
}
function setDog(newDog) {
    dog = newDog;
}
const dogRef = {
    initial: dog,
    get: getDog,
    set: setDog
};
export { getDog } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { setDog } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { dogRef } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 3
```js
import { cat } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 12
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 10
};
export { getChimera };
function getChimera() {
    return cat + dog;
}
export { getChimera } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 4
```js
import { cat } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 12
};
export { initialCat };
const initialCat = cat;
export { initialCat } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 5
```js
let dog = "dog";
export { dog } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 6
```js
import { dog } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
dog += "!";

```
## Part 7
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 6
};
import { dog } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 1
};
console.log(dog);

```
## Part 8
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 6
};
import { dog } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 7
};
dog += "!";

```
## Part 9
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 8
};
import { dog } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 7
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 1
};
console.log(dog);

```
## Part 10
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 8
};
import { dog } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 9
};
dog += "!";

```
## Part 11
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 10
};
import { dog } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 7
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 9
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 1
};
console.log(dog);

```
## Part 12
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 1
};
let cat = "cat";
export { cat } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Merged (module eval)
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 7
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 9
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 11
};
"module evaluation";

```
# Entrypoints

```
{
    ModuleEvaluation: 0,
    Export(
        "getChimera",
    ): 3,
    Export(
        "initialCat",
    ): 4,
    Export(
        "cat",
    ): 1,
    Export(
        "dogRef",
    ): 2,
}
```


# Modules (prod)
## Part 0
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 6
};
import { dog } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 7
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 8
};
"module evaluation";
console.log(dog);
console.log(dog);
console.log(dog);

```
## Part 1
```js
import { cat } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 9
};
export { cat };

```
## Part 2
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 8
};
import { dog } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
export { dogRef };
function getDog() {
    return dog;
}
function setDog(newDog) {
    dog = newDog;
}
const dogRef = {
    initial: dog,
    get: getDog,
    set: setDog
};
export { getDog } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { setDog } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { dogRef } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 3
```js
import { cat } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 9
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 8
};
export { getChimera };
function getChimera() {
    return cat + dog;
}
export { getChimera } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 4
```js
import { cat } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 9
};
export { initialCat };
const initialCat = cat;
export { initialCat } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 5
```js
let dog = "dog";
export { dog } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 6
```js
import { dog } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
dog += "!";

```
## Part 7
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 6
};
import { dog } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
dog += "!";

```
## Part 8
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 7
};
import { dog } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
dog += "!";

```
## Part 9
```js
let cat = "cat";
export { cat } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Merged (module eval)
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 6
};
import { dog } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 7
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 8
};
"module evaluation";
console.log(dog);
console.log(dog);
console.log(dog);

```
