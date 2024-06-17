# Items

Count: 10

## Item 5: Stmt 0, `VarDeclarator(0)`

```js
let source;

```

- Declares: `source`
- Write: `source`

## Item 6: Stmt 1, `VarDeclarator(0)`

```js
const eventCallbacks = [];

```

- Declares: `eventCallbacks`
- Write: `eventCallbacks`

## Item 7: Stmt 2, `Normal`

```js
function getSocketProtocol(assetPrefix) {
    let protocol = location.protocol;
    try {
        protocol = new URL(assetPrefix).protocol;
    } catch (_) {}
    return protocol === "http:" ? "ws" : "wss";
}

```

- Hoisted
- Declares: `getSocketProtocol`
- Write: `getSocketProtocol`

## Item 8: Stmt 3, `Normal`

```js
export function addMessageListener(cb) {
    eventCallbacks.push(cb);
}

```

- Hoisted
- Declares: `addMessageListener`
- Reads (eventual): `eventCallbacks`
- Write: `addMessageListener`
- Write (eventual): `eventCallbacks`

## Item 9: Stmt 4, `Normal`

```js
export function sendMessage(data) {
    if (!source || source.readyState !== source.OPEN) return;
    return source.send(data);
}

```

- Hoisted
- Declares: `sendMessage`
- Reads (eventual): `source`
- Write: `sendMessage`
- Write (eventual): `source`

## Item 10: Stmt 5, `Normal`

```js
export function connectHMR(options) {
    const { timeout = 5 * 1000 } = options;
    function init() {
        if (source) source.close();
        console.log("[HMR] connecting...");
        function handleOnline() {
            const connected = {
                type: "turbopack-connected"
            };
            eventCallbacks.forEach((cb)=>{
                cb(connected);
            });
            if (options.log) console.log("[HMR] connected");
        }
        function handleMessage(event) {
            const message = {
                type: "turbopack-message",
                data: JSON.parse(event.data)
            };
            eventCallbacks.forEach((cb)=>{
                cb(message);
            });
        }
        function handleDisconnect() {
            source.close();
            setTimeout(init, timeout);
        }
        const { hostname, port } = location;
        const protocol = getSocketProtocol(options.assetPrefix || "");
        const assetPrefix = options.assetPrefix.replace(/^\/+/, "");
        let url = `${protocol}://${hostname}:${port}${assetPrefix ? `/${assetPrefix}` : ""}`;
        if (assetPrefix.startsWith("http")) {
            url = `${protocol}://${assetPrefix.split("://")[1]}`;
        }
        source = new window.WebSocket(`${url}${options.path}`);
        source.onopen = handleOnline;
        source.onerror = handleDisconnect;
        source.onmessage = handleMessage;
    }
    init();
}

```

- Hoisted
- Declares: `connectHMR`
- Reads (eventual): `source`, `eventCallbacks`, `getSocketProtocol`
- Write: `connectHMR`
- Write (eventual): `source`, `eventCallbacks`

# Phase 1
```mermaid
graph TD
    Item1;
    Item1["ModuleEvaluation"];
    Item2;
    Item2["export addMessageListener"];
    Item3;
    Item3["export connectHMR"];
    Item4;
    Item4["export sendMessage"];
    Item5;
    Item6;
    Item7;
    Item8;
    Item9;
    Item10;
```
# Phase 2
```mermaid
graph TD
    Item1;
    Item1["ModuleEvaluation"];
    Item2;
    Item2["export addMessageListener"];
    Item3;
    Item3["export connectHMR"];
    Item4;
    Item4["export sendMessage"];
    Item5;
    Item6;
    Item7;
    Item8;
    Item9;
    Item10;
    Item2 --> Item8;
    Item3 --> Item10;
    Item4 --> Item9;
```
# Phase 3
```mermaid
graph TD
    Item1;
    Item1["ModuleEvaluation"];
    Item2;
    Item2["export addMessageListener"];
    Item3;
    Item3["export connectHMR"];
    Item4;
    Item4["export sendMessage"];
    Item5;
    Item6;
    Item7;
    Item8;
    Item9;
    Item10;
    Item2 --> Item8;
    Item3 --> Item10;
    Item4 --> Item9;
    Item8 --> Item6;
    Item9 --> Item5;
    Item10 --> Item5;
    Item10 --> Item6;
    Item10 --> Item7;
```
# Phase 4
```mermaid
graph TD
    Item1;
    Item1["ModuleEvaluation"];
    Item2;
    Item2["export addMessageListener"];
    Item3;
    Item3["export connectHMR"];
    Item4;
    Item4["export sendMessage"];
    Item5;
    Item6;
    Item7;
    Item8;
    Item9;
    Item10;
    Item2 --> Item8;
    Item3 --> Item10;
    Item4 --> Item9;
    Item8 --> Item6;
    Item9 --> Item5;
    Item10 --> Item5;
    Item10 --> Item6;
    Item10 --> Item7;
```
# Final
```mermaid
graph TD
    N0["Items: [ItemId(ModuleEvaluation)]"];
    N1["Items: [ItemId(Export((&quot;addMessageListener&quot;, #2), &quot;addMessageListener&quot;)), ItemId(3, Normal)]"];
    N2["Items: [ItemId(Export((&quot;connectHMR&quot;, #2), &quot;connectHMR&quot;)), ItemId(2, Normal), ItemId(5, Normal)]"];
    N3["Items: [ItemId(Export((&quot;sendMessage&quot;, #2), &quot;sendMessage&quot;)), ItemId(4, Normal)]"];
    N4["Items: [ItemId(0, VarDeclarator(0))]"];
    N5["Items: [ItemId(1, VarDeclarator(0))]"];
    N1 --> N5;
    N2 --> N4;
    N2 --> N5;
    N3 --> N4;
```
# Entrypoints

```
{
    ModuleEvaluation: 0,
    Export(
        "connectHMR",
    ): 2,
    Export(
        "addMessageListener",
    ): 1,
    Export(
        "sendMessage",
    ): 3,
}
```


# Modules (dev)
## Part 0
```js
"module evaluation";

```
## Part 1
```js
import { eventCallbacks } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
export { addMessageListener };
function addMessageListener(cb) {
    eventCallbacks.push(cb);
}
export { addMessageListener } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 2
```js
import { source } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 4
};
import { eventCallbacks } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
export { connectHMR };
function getSocketProtocol(assetPrefix) {
    let protocol = location.protocol;
    try {
        protocol = new URL(assetPrefix).protocol;
    } catch (_) {}
    return protocol === "http:" ? "ws" : "wss";
}
function connectHMR(options) {
    const { timeout = 5 * 1000 } = options;
    function init() {
        if (source) source.close();
        console.log("[HMR] connecting...");
        function handleOnline() {
            const connected = {
                type: "turbopack-connected"
            };
            eventCallbacks.forEach((cb)=>{
                cb(connected);
            });
            if (options.log) console.log("[HMR] connected");
        }
        function handleMessage(event) {
            const message = {
                type: "turbopack-message",
                data: JSON.parse(event.data)
            };
            eventCallbacks.forEach((cb)=>{
                cb(message);
            });
        }
        function handleDisconnect() {
            source.close();
            setTimeout(init, timeout);
        }
        const { hostname, port } = location;
        const protocol = getSocketProtocol(options.assetPrefix || "");
        const assetPrefix = options.assetPrefix.replace(/^\/+/, "");
        let url = `${protocol}://${hostname}:${port}${assetPrefix ? `/${assetPrefix}` : ""}`;
        if (assetPrefix.startsWith("http")) {
            url = `${protocol}://${assetPrefix.split("://")[1]}`;
        }
        source = new window.WebSocket(`${url}${options.path}`);
        source.onopen = handleOnline;
        source.onerror = handleDisconnect;
        source.onmessage = handleMessage;
    }
    init();
}
export { getSocketProtocol } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { connectHMR } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 3
```js
import { source } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 4
};
export { sendMessage };
function sendMessage(data) {
    if (!source || source.readyState !== source.OPEN) return;
    return source.send(data);
}
export { sendMessage } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 4
```js
let source;
export { source } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 5
```js
const eventCallbacks = [];
export { eventCallbacks } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

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
        "connectHMR",
    ): 2,
    Export(
        "addMessageListener",
    ): 1,
    Export(
        "sendMessage",
    ): 3,
}
```


# Modules (prod)
## Part 0
```js
"module evaluation";

```
## Part 1
```js
import { eventCallbacks } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
export { addMessageListener };
function addMessageListener(cb) {
    eventCallbacks.push(cb);
}
export { addMessageListener } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 2
```js
import { source } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 4
};
import { eventCallbacks } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 5
};
export { connectHMR };
function getSocketProtocol(assetPrefix) {
    let protocol = location.protocol;
    try {
        protocol = new URL(assetPrefix).protocol;
    } catch (_) {}
    return protocol === "http:" ? "ws" : "wss";
}
function connectHMR(options) {
    const { timeout = 5 * 1000 } = options;
    function init() {
        if (source) source.close();
        console.log("[HMR] connecting...");
        function handleOnline() {
            const connected = {
                type: "turbopack-connected"
            };
            eventCallbacks.forEach((cb)=>{
                cb(connected);
            });
            if (options.log) console.log("[HMR] connected");
        }
        function handleMessage(event) {
            const message = {
                type: "turbopack-message",
                data: JSON.parse(event.data)
            };
            eventCallbacks.forEach((cb)=>{
                cb(message);
            });
        }
        function handleDisconnect() {
            source.close();
            setTimeout(init, timeout);
        }
        const { hostname, port } = location;
        const protocol = getSocketProtocol(options.assetPrefix || "");
        const assetPrefix = options.assetPrefix.replace(/^\/+/, "");
        let url = `${protocol}://${hostname}:${port}${assetPrefix ? `/${assetPrefix}` : ""}`;
        if (assetPrefix.startsWith("http")) {
            url = `${protocol}://${assetPrefix.split("://")[1]}`;
        }
        source = new window.WebSocket(`${url}${options.path}`);
        source.onopen = handleOnline;
        source.onerror = handleDisconnect;
        source.onmessage = handleMessage;
    }
    init();
}
export { getSocketProtocol } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};
export { connectHMR } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 3
```js
import { source } from "__TURBOPACK_PART__" assert {
    __turbopack_part__: 4
};
export { sendMessage };
function sendMessage(data) {
    if (!source || source.readyState !== source.OPEN) return;
    return source.send(data);
}
export { sendMessage } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 4
```js
let source;
export { source } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Part 5
```js
const eventCallbacks = [];
export { eventCallbacks } from "__TURBOPACK_VAR__" assert {
    __turbopack_var__: true
};

```
## Merged (module eval)
```js
"module evaluation";

```
