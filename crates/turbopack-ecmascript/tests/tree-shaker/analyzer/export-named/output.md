# Items

Count: 4

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

# Phase 1
```mermaid
graph TD
    Item1;
    Item2;
    Item3;
    Item3["ModuleEvaluation"];
    Item4;
    Item4["export fakeCat"];
```
# Phase 2
```mermaid
graph TD
    Item1;
    Item2;
    Item3;
    Item3["ModuleEvaluation"];
    Item4;
    Item4["export fakeCat"];
```
# Phase 3
```mermaid
graph TD
    Item1;
    Item2;
    Item3;
    Item3["ModuleEvaluation"];
    Item4;
    Item4["export fakeCat"];
```
# Phase 4
```mermaid
graph TD
    Item1;
    Item2;
    Item3;
    Item3["ModuleEvaluation"];
    Item4;
    Item4["export fakeCat"];
    Item3 --> Item1;
    Item4 --> Item2;
```
# Final
```mermaid
graph TD
    N0["Items: [ItemId(ModuleEvaluation), ItemId(0, ImportOfModule)]"];
    N1["Items: [ItemId(Export((&quot;_reexport_fakeCat&quot;, #1), &quot;fakeCat&quot;)), ItemId(0, ImportBinding(0))]"];
```
# Modules (dev)
## Part 0
```js
"module evaluation";
import "./lib";

```
## Part 1
```js
export { _reexport_fakeCat as fakeCat };
import { cat as _reexport_fakeCat } from "./lib";

```
## Merged (module eval)
```js
import "./lib";
"module evaluation";

```
# Modules (prod)
## Part 0
```js
"module evaluation";
import "./lib";

```
## Part 1
```js
export { _reexport_fakeCat as fakeCat };
import { cat as _reexport_fakeCat } from "./lib";

```
## Merged (module eval)
```js
import "./lib";
"module evaluation";

```
