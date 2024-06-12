# Items

Count: 12

## Item 1: Stmt 0, `Normal`

```js
console.log("Hello");

```

- Side effects

## Item 2: Stmt 1, `VarDeclarator(0)`

```js
const value = externalFunction();

```

- Side effects
- Declares: `value`
- Write: `value`

## Item 3: Stmt 2, `VarDeclarator(0)`

```js
const value2 = externalObject.propertyWithGetter;

```

- Side effects
- Declares: `value2`
- Write: `value2`

## Item 4: Stmt 3, `Normal`

```js
externalObject.propertyWithSetter = 42;

```

- Side effects

## Item 5: Stmt 4, `VarDeclarator(0)`

```js
const value3 = externalFunction();

```

- Side effects
- Declares: `value3`
- Write: `value3`

## Item 6: Stmt 5, `VarDeclarator(0)`

```js
const shared = {
    value,
    value2,
    value3
};

```

- Declares: `shared`
- Reads: `value`, `value2`, `value3`
- Write: `shared`

## Item 7: Stmt 6, `Normal`

```js
console.log(shared);

```

- Side effects
- Reads: `shared`

## Item 8: Stmt 7, `VarDeclarator(0)`

```js
export const a = {
    shared,
    a: "aaaaaaaaaaa"
};

```

- Declares: `a`
- Reads: `shared`
- Write: `a`

## Item 9: Stmt 8, `VarDeclarator(0)`

```js
export const b = {
    shared,
    b: "bbbbbbbbbbb"
};

```

- Declares: `b`
- Reads: `shared`
- Write: `b`

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
    Item10["ModuleEvaluation"];
    Item11;
    Item11["export a"];
    Item12;
    Item12["export b"];
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
    Item10["ModuleEvaluation"];
    Item11;
    Item11["export a"];
    Item12;
    Item12["export b"];
    Item2 -.-> Item2;
    Item2 --> Item1;
    Item3 -.-> Item3;
    Item3 --> Item1;
    Item3 --> Item2;
    Item4 --> Item1;
    Item4 --> Item2;
    Item4 --> Item3;
    Item5 -.-> Item5;
    Item5 --> Item1;
    Item5 --> Item2;
    Item5 --> Item3;
    Item5 --> Item4;
    Item6 --> Item2;
    Item6 --> Item3;
    Item6 --> Item5;
    Item6 -.-> Item6;
    Item7 --> Item6;
    Item7 --> Item1;
    Item7 --> Item2;
    Item7 --> Item3;
    Item7 --> Item4;
    Item7 --> Item5;
    Item8 --> Item6;
    Item8 -.-> Item8;
    Item9 --> Item6;
    Item9 -.-> Item9;
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
    Item10["ModuleEvaluation"];
    Item11;
    Item11["export a"];
    Item12;
    Item12["export b"];
    Item2 -.-> Item2;
    Item2 --> Item1;
    Item3 -.-> Item3;
    Item3 --> Item1;
    Item3 --> Item2;
    Item4 --> Item1;
    Item4 --> Item2;
    Item4 --> Item3;
    Item5 -.-> Item5;
    Item5 --> Item1;
    Item5 --> Item2;
    Item5 --> Item3;
    Item5 --> Item4;
    Item6 --> Item2;
    Item6 --> Item3;
    Item6 --> Item5;
    Item6 -.-> Item6;
    Item7 --> Item6;
    Item7 --> Item1;
    Item7 --> Item2;
    Item7 --> Item3;
    Item7 --> Item4;
    Item7 --> Item5;
    Item8 --> Item6;
    Item8 -.-> Item8;
    Item9 --> Item6;
    Item9 -.-> Item9;
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
    Item10["ModuleEvaluation"];
    Item11;
    Item11["export a"];
    Item12;
    Item12["export b"];
    Item2 -.-> Item2;
    Item2 --> Item1;
    Item3 -.-> Item3;
    Item3 --> Item1;
    Item3 --> Item2;
    Item4 --> Item1;
    Item4 --> Item2;
    Item4 --> Item3;
    Item5 -.-> Item5;
    Item5 --> Item1;
    Item5 --> Item2;
    Item5 --> Item3;
    Item5 --> Item4;
    Item6 --> Item2;
    Item6 --> Item3;
    Item6 --> Item5;
    Item6 -.-> Item6;
    Item7 --> Item6;
    Item7 --> Item1;
    Item7 --> Item2;
    Item7 --> Item3;
    Item7 --> Item4;
    Item7 --> Item5;
    Item8 --> Item6;
    Item8 -.-> Item8;
    Item9 --> Item6;
    Item9 -.-> Item9;
    Item10 --> Item1;
    Item10 --> Item2;
    Item10 --> Item3;
    Item10 --> Item4;
    Item10 --> Item5;
    Item10 --> Item7;
```
# Final
```mermaid
graph TD
    N0["Items: [ItemId(ModuleEvaluation), ItemId(0, Normal), ItemId(1, VarDeclarator(0)), ItemId(2, VarDeclarator(0)), ItemId(3, Normal), ItemId(4, VarDeclarator(0)), ItemId(5, VarDeclarator(0)), ItemId(6, Normal)]"];
    N1["Items: [ItemId(Export((&quot;a&quot;, #2), &quot;a&quot;))]"];
    N2["Items: [ItemId(Export((&quot;b&quot;, #2), &quot;b&quot;))]"];
```
# Entrypoints

```
{
    ModuleEvaluation: 0,
    Export(
        "b",
    ): 2,
    Export(
        "a",
    ): 1,
}
```


# Modules (dev)
## Part 0
```js
"module evaluation";
console.log("Hello");
const value = externalFunction();
const value2 = externalObject.propertyWithGetter;
externalObject.propertyWithSetter = 42;
const value3 = externalFunction();
const shared = {
    value,
    value2,
    value3
};
console.log(shared);

```
## Part 1
```js
export { a };

```
## Part 2
```js
export { b };

```
## Merged (module eval)
```js
"module evaluation";
console.log("Hello");
const value = externalFunction();
const value2 = externalObject.propertyWithGetter;
externalObject.propertyWithSetter = 42;
const value3 = externalFunction();
const shared = {
    value,
    value2,
    value3
};
console.log(shared);

```
# Entrypoints

```
{
    ModuleEvaluation: 0,
    Export(
        "b",
    ): 2,
    Export(
        "a",
    ): 1,
}
```


# Modules (prod)
## Part 0
```js
"module evaluation";
console.log("Hello");
const value = externalFunction();
const value2 = externalObject.propertyWithGetter;
externalObject.propertyWithSetter = 42;
const value3 = externalFunction();
const shared = {
    value,
    value2,
    value3
};
console.log(shared);

```
## Part 1
```js
export { a };

```
## Part 2
```js
export { b };

```
## Merged (module eval)
```js
"module evaluation";
console.log("Hello");
const value = externalFunction();
const value2 = externalObject.propertyWithGetter;
externalObject.propertyWithSetter = 42;
const value3 = externalFunction();
const shared = {
    value,
    value2,
    value3
};
console.log(shared);

```
