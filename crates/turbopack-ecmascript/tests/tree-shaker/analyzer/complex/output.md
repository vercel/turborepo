# Items

Count: 18

## Item 1: Stmt 0, `VarDeclarator(0)`

```js
let dog = "dog";

```

- Declares: `dog`
- Write: `dog`

## Item 2: Stmt 1, `Normal`

```js
dog += "!";

```

- Reads: `dog`
- Write: `dog`

## Item 3: Stmt 2, `Normal`

```js
console.log(dog);

```

- Side effects
- Reads: `dog`

## Item 4: Stmt 3, `Normal`

```js
function getDog() {
    return dog;
}

```

- Hoisted
- Declares: `getDog`
- Reads (eventual): `dog`
- Write: `getDog`

## Item 5: Stmt 4, `Normal`

```js
dog += "!";

```

- Reads: `dog`
- Write: `dog`

## Item 6: Stmt 5, `Normal`

```js
console.log(dog);

```

- Side effects
- Reads: `dog`

## Item 7: Stmt 6, `Normal`

```js
function setDog(newDog) {
    dog = newDog;
}

```

- Hoisted
- Declares: `setDog`
- Write: `setDog`
- Write (eventual): `dog`

## Item 8: Stmt 7, `Normal`

```js
dog += "!";

```

- Reads: `dog`
- Write: `dog`

## Item 9: Stmt 8, `Normal`

```js
console.log(dog);

```

- Side effects
- Reads: `dog`

## Item 10: Stmt 9, `VarDeclarator(0)`

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

## Item 11: Stmt 10, `VarDeclarator(0)`

```js
export let cat = "cat";

```

- Declares: `cat`
- Write: `cat`

## Item 12: Stmt 11, `VarDeclarator(0)`

```js
export const initialCat = cat;

```

- Declares: `initialCat`
- Reads: `cat`
- Write: `initialCat`

## Item 13: Stmt 12, `Normal`

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
    Item1 -.-> Item1;
    Item2 -.-> Item1;
    Item3 --> Item2;
    Item5 --> Item2;
    Item5 -.-> Item3;
    Item5 -.-> Item1;
    Item6 --> Item5;
    Item6 --> Item3;
    Item8 --> Item5;
    Item8 -.-> Item6;
    Item8 -.-> Item1;
    Item9 --> Item8;
    Item9 --> Item3;
    Item9 --> Item6;
    Item10 --> Item8;
    Item10 --> Item4;
    Item10 --> Item7;
    Item10 -.-> Item10;
    Item11 -.-> Item11;
    Item12 -.-> Item12;
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
    Item1 -.-> Item1;
    Item2 -.-> Item1;
    Item3 --> Item2;
    Item5 --> Item2;
    Item5 -.-> Item3;
    Item5 -.-> Item1;
    Item6 --> Item5;
    Item6 --> Item3;
    Item8 --> Item5;
    Item8 -.-> Item6;
    Item8 -.-> Item1;
    Item9 --> Item8;
    Item9 --> Item3;
    Item9 --> Item6;
    Item10 --> Item8;
    Item10 --> Item4;
    Item10 --> Item7;
    Item10 -.-> Item10;
    Item11 -.-> Item11;
    Item12 -.-> Item12;
    Item4 --> Item8;
    Item7 -.-> Item9;
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
    Item1 -.-> Item1;
    Item2 -.-> Item1;
    Item3 --> Item2;
    Item5 --> Item2;
    Item5 -.-> Item3;
    Item5 -.-> Item1;
    Item6 --> Item5;
    Item6 --> Item3;
    Item8 --> Item5;
    Item8 -.-> Item6;
    Item8 -.-> Item1;
    Item9 --> Item8;
    Item9 --> Item3;
    Item9 --> Item6;
    Item10 --> Item8;
    Item10 --> Item4;
    Item10 --> Item7;
    Item10 -.-> Item10;
    Item11 -.-> Item11;
    Item12 -.-> Item12;
    Item4 --> Item8;
    Item7 -.-> Item9;
    Item13 --> Item8;
    Item14 --> Item3;
    Item14 --> Item6;
    Item14 --> Item9;
    Item18 --> Item13;
```
# Final
```mermaid
graph TD
    N0["Items: [ItemId(ModuleEvaluation), ItemId(8, Normal)]"];
    N1["Items: [ItemId(Export((&quot;dogRef&quot;, #2), &quot;dogRef&quot;))]"];
    N2["Items: [ItemId(Export((&quot;cat&quot;, #2), &quot;cat&quot;))]"];
    N3["Items: [ItemId(Export((&quot;initialCat&quot;, #2), &quot;initialCat&quot;))]"];
    N4["Items: [ItemId(Export((&quot;getChimera&quot;, #2), &quot;getChimera&quot;)), ItemId(12, Normal)]"];
    N5["Items: [ItemId(0, VarDeclarator(0))]"];
    N6["Items: [ItemId(1, Normal)]"];
    N7["Items: [ItemId(2, Normal)]"];
    N8["Items: [ItemId(4, Normal)]"];
    N9["Items: [ItemId(5, Normal)]"];
    N10["Items: [ItemId(7, Normal)]"];
    N0 --> N7;
    N0 --> N9;
    N0 --> N10;
    N4 --> N10;
    N6 --> N5;
    N7 --> N6;
    N8 --> N6;
    N8 --> N7;
    N8 --> N5;
    N9 --> N8;
    N9 --> N7;
    N10 --> N8;
    N10 --> N9;
    N10 --> N5;
```
# Entrypoints

```
{
    ModuleEvaluation: 0,
    Export(
        "getChimera",
    ): 4,
    Export(
        "initialCat",
    ): 3,
    Export(
        "cat",
    ): 2,
    Export(
        "dogRef",
    ): 1,
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
    __turbopack_part__: 10
};
"module evaluation";
console.log(dog);

```
## Part 1
```js
export { dogRef };

```
## Part 2
```js
export { cat };

```
## Part 3
```js
export { initialCat };

```
## Part 4
```js
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
console.log(dog);

```
## Part 8
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 6
};
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
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 8
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 7
};
console.log(dog);

```
## Part 10
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 8
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 9
};
import { dog } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
dog += "!";

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
    __turbopack_part__: 10
};
"module evaluation";
console.log(dog);

```
# Entrypoints

```
{
    ModuleEvaluation: 0,
    Export(
        "getChimera",
    ): 4,
    Export(
        "initialCat",
    ): 3,
    Export(
        "cat",
    ): 2,
    Export(
        "dogRef",
    ): 1,
}
```


# Modules (prod)
## Part 0
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 6
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 7
};
"module evaluation";
console.log(dog);
console.log(dog);
console.log(dog);

```
## Part 1
```js
export { dogRef };

```
## Part 2
```js
export { cat };

```
## Part 3
```js
export { initialCat };

```
## Part 4
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 7
};
export { getChimera };
function getChimera() {
    return cat + dog;
}
export { getChimera } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 5
```js
dog += "!";

```
## Part 6
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
dog += "!";

```
## Part 7
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 6
};
dog += "!";

```
## Merged (module eval)
```js
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 6
};
import "__TURBOPACK_PART__" assert {
    __turbopack_part__: 7
};
"module evaluation";
console.log(dog);
console.log(dog);
console.log(dog);

```
