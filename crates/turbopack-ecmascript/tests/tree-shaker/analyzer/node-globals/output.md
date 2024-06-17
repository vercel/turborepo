# Items

Count: 2

## Item 2: Stmt 0, `Normal`

```js
process.turbopack = {};

```

- Side effects

# Phase 1
```mermaid
graph TD
    Item1;
    Item1["ModuleEvaluation"];
    Item2;
```
# Phase 2
```mermaid
graph TD
    Item1;
    Item1["ModuleEvaluation"];
    Item2;
```
# Phase 3
```mermaid
graph TD
    Item1;
    Item1["ModuleEvaluation"];
    Item2;
```
# Phase 4
```mermaid
graph TD
    Item1;
    Item1["ModuleEvaluation"];
    Item2;
    Item1 --> Item2;
```
# Final
```mermaid
graph TD
    N0["Items: [ItemId(ModuleEvaluation), ItemId(0, Normal)]"];
```
# Entrypoints

```
{
    ModuleEvaluation: 0,
}
```


# Modules (dev)
## Part 0
```js
"module evaluation";
process.turbopack = {};

```
## Merged (module eval)
```js
"module evaluation";
process.turbopack = {};

```
# Entrypoints

```
{
    ModuleEvaluation: 0,
}
```


# Modules (prod)
## Part 0
```js
"module evaluation";
process.turbopack = {};

```
## Merged (module eval)
```js
"module evaluation";
process.turbopack = {};

```
