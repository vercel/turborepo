# Items

Count: 6

## Item 1: Stmt 0, `ImportOfModule`

```js
export { cat as fakeCat } from "./lib";

```

- Hoisted
- Side effects

## Item 2: Stmt 0, `ImportBinding(0)`

```js
export { cat as fakeCat } from "./lib";

```

- Hoisted
- Declares: `_reexport_fakeCat`

## Item 3: Stmt 0, `Normal`

```js
export { cat as fakeCat } from "./lib";

```

- Side effects
- Reads: `_reexport_fakeCat`

## Item 4: Stmt 1, `Normal`

```js
console.log("done");

```

- Side effects
- Reads: `console`

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
    Item6["export fakeCat"];
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
    Item6["export fakeCat"];
    Item3 --> Item2;
    Item3 --> Item1;
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
    Item6["export fakeCat"];
    Item3 --> Item2;
    Item3 --> Item1;
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
    Item6["export fakeCat"];
    Item3 --> Item2;
    Item3 --> Item1;
    Item4 --> Item1;
    Item4 --> Item3;
    Item5 --> Item1;
    Item5 --> Item3;
    Item5 --> Item4;
```
# Final
```mermaid
graph TD
    N0["Items: [ItemId(ModuleEvaluation), ItemId(0, Normal), ItemId(0, ImportOfModule), ItemId(0, ImportBinding(0)), ItemId(1, Normal)]"];
    N1["Items: [ItemId(Export((\"fakeCat\", #0)))]"];
```
# Modules (dev)
## Part 0
```js
"module evaluation";
export { cat as fakeCat } from "./lib";
import "./lib";
import { cat as _reexport_fakeCat } from "./lib";
console.log("done");

```
## Part 1
```js
export { _reexport_fakeCat as fakeCat };

```
## Merged (module eval)
```js
import "./lib";
import { cat as _reexport_fakeCat } from "./lib";
"module evaluation";
export { cat as fakeCat } from "./lib";
console.log("done");

```
# Modules (prod)
## Part 0
```js
"module evaluation";
export { cat as fakeCat } from "./lib";
import "./lib";
import { cat as _reexport_fakeCat } from "./lib";
console.log("done");

```
## Part 1
```js
export { _reexport_fakeCat as fakeCat };

```
## Merged (module eval)
```js
import "./lib";
import { cat as _reexport_fakeCat } from "./lib";
"module evaluation";
export { cat as fakeCat } from "./lib";
console.log("done");

```
