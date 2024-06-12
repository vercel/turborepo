# Items

Count: 13

## Item 1: Stmt 0, `VarDeclarator(0)`

```js
let x = 1;

```

- Declares: `x`
- Write: `x`

## Item 2: Stmt 1, `Normal`

```js
x = 2;

```

- Write: `x`

## Item 3: Stmt 2, `Normal`

```js
x = 3;

```

- Write: `x`

## Item 4: Stmt 3, `Normal`

```js
x = 4;

```

- Write: `x`

## Item 5: Stmt 4, `Normal`

```js
x = 5;

```

- Write: `x`

## Item 6: Stmt 5, `Normal`

```js
x += 6;

```

- Reads: `x`
- Write: `x`

## Item 7: Stmt 6, `Normal`

```js
x += 7;

```

- Reads: `x`
- Write: `x`

## Item 8: Stmt 7, `Normal`

```js
x += 8;

```

- Reads: `x`
- Write: `x`

## Item 9: Stmt 8, `Normal`

```js
x += 9;

```

- Reads: `x`
- Write: `x`

## Item 10: Stmt 10, `VarDeclarator(0)`

```js
export const y = x;

```

- Declares: `y`
- Reads: `x`
- Write: `y`

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
    Item11["ModuleEvaluation"];
    Item12;
    Item12["export x"];
    Item13;
    Item13["export y"];
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
    Item11["ModuleEvaluation"];
    Item12;
    Item12["export x"];
    Item13;
    Item13["export y"];
    Item1 -.-> Item1;
    Item2 -.-> Item1;
    Item3 -.-> Item1;
    Item4 -.-> Item1;
    Item5 -.-> Item1;
    Item6 --> Item1;
    Item7 --> Item1;
    Item7 -.-> Item6;
    Item8 --> Item1;
    Item8 -.-> Item7;
    Item9 --> Item1;
    Item9 -.-> Item8;
    Item10 --> Item1;
    Item10 -.-> Item10;
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
    Item11["ModuleEvaluation"];
    Item12;
    Item12["export x"];
    Item13;
    Item13["export y"];
    Item1 -.-> Item1;
    Item2 -.-> Item1;
    Item3 -.-> Item1;
    Item4 -.-> Item1;
    Item5 -.-> Item1;
    Item6 --> Item1;
    Item7 --> Item1;
    Item7 -.-> Item6;
    Item8 --> Item1;
    Item8 -.-> Item7;
    Item9 --> Item1;
    Item9 -.-> Item8;
    Item10 --> Item1;
    Item10 -.-> Item10;
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
    Item11["ModuleEvaluation"];
    Item12;
    Item12["export x"];
    Item13;
    Item13["export y"];
    Item1 -.-> Item1;
    Item2 -.-> Item1;
    Item3 -.-> Item1;
    Item4 -.-> Item1;
    Item5 -.-> Item1;
    Item6 --> Item1;
    Item7 --> Item1;
    Item7 -.-> Item6;
    Item8 --> Item1;
    Item8 -.-> Item7;
    Item9 --> Item1;
    Item9 -.-> Item8;
    Item10 --> Item1;
    Item10 -.-> Item10;
```
# Final
```mermaid
graph TD
    N0["Items: [ItemId(ModuleEvaluation)]"];
    N1["Items: [ItemId(Export((&quot;x&quot;, #2), &quot;x&quot;))]"];
    N2["Items: [ItemId(Export((&quot;y&quot;, #2), &quot;y&quot;))]"];
```
# Entrypoints

```
{
    ModuleEvaluation: 0,
    Export(
        "y",
    ): 2,
    Export(
        "x",
    ): 1,
}
```


# Modules (dev)
## Part 0
```js
"module evaluation";

```
## Part 1
```js
export { x as x };

```
## Part 2
```js
export { y };

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
        "y",
    ): 2,
    Export(
        "x",
    ): 1,
}
```


# Modules (prod)
## Part 0
```js
"module evaluation";

```
## Part 1
```js
export { x as x };

```
## Part 2
```js
export { y };

```
## Merged (module eval)
```js
"module evaluation";

```
