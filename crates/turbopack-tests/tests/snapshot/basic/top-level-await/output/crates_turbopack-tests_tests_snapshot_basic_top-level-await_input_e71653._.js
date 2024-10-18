(globalThis.TURBOPACK = globalThis.TURBOPACK || []).push(["output/crates_turbopack-tests_tests_snapshot_basic_top-level-await_input_e71653._.js", {

"[project]/crates/turbopack-tests/tests/snapshot/basic/top-level-await/input/db-connection.js [test] (ecmascript)": (({ r: __turbopack_require__, f: __turbopack_module_context__, i: __turbopack_import__, s: __turbopack_esm__, v: __turbopack_export_value__, n: __turbopack_export_namespace__, c: __turbopack_cache__, M: __turbopack_modules__, l: __turbopack_load__, j: __turbopack_dynamic__, P: __turbopack_resolve_absolute_path__, U: __turbopack_relative_url__, R: __turbopack_resolve_module_id_path__, g: global, __dirname, a: __turbopack_async_module__ }) => (() => {
"use strict";

__turbopack_async_module__(async (__turbopack_handle_async_dependencies__, __turbopack_async_result__) => { try {
__turbopack_esm__({
    "close": ()=>close,
    "dbCall": ()=>dbCall
});
const connectToDB = async (url)=>{
    console.log("connecting to db", url);
    await new Promise((r)=>setTimeout(r, 1000));
};
// This is a top-level-await
await connectToDB("my-sql://example.com");
const dbCall = async (data)=>{
    console.log("dbCall", data);
    // This is a normal await, because it's in an async function
    await new Promise((r)=>setTimeout(r, 100));
    return "fake data";
};
const close = ()=>{
    console.log("closes the DB connection");
};
__turbopack_async_result__();
} catch(e) { __turbopack_async_result__(e); } }, true);
})()),
"[project]/crates/turbopack-tests/tests/snapshot/basic/top-level-await/input/UserAPI.js [test] (ecmascript)": (({ r: __turbopack_require__, f: __turbopack_module_context__, i: __turbopack_import__, s: __turbopack_esm__, v: __turbopack_export_value__, n: __turbopack_export_namespace__, c: __turbopack_cache__, M: __turbopack_modules__, l: __turbopack_load__, j: __turbopack_dynamic__, P: __turbopack_resolve_absolute_path__, U: __turbopack_relative_url__, R: __turbopack_resolve_module_id_path__, g: global, __dirname, a: __turbopack_async_module__ }) => (() => {
"use strict";

__turbopack_async_module__(async (__turbopack_handle_async_dependencies__, __turbopack_async_result__) => { try {
__turbopack_esm__({
    "createUser": ()=>createUser
});
var __TURBOPACK__imported__module__$5b$project$5d2f$crates$2f$turbopack$2d$tests$2f$tests$2f$snapshot$2f$basic$2f$top$2d$level$2d$await$2f$input$2f$db$2d$connection$2e$js__$5b$test$5d$__$28$ecmascript$29$__ = __turbopack_import__("[project]/crates/turbopack-tests/tests/snapshot/basic/top-level-await/input/db-connection.js [test] (ecmascript)");
var __turbopack_async_dependencies__ = __turbopack_handle_async_dependencies__([
    __TURBOPACK__imported__module__$5b$project$5d2f$crates$2f$turbopack$2d$tests$2f$tests$2f$snapshot$2f$basic$2f$top$2d$level$2d$await$2f$input$2f$db$2d$connection$2e$js__$5b$test$5d$__$28$ecmascript$29$__
]);
[__TURBOPACK__imported__module__$5b$project$5d2f$crates$2f$turbopack$2d$tests$2f$tests$2f$snapshot$2f$basic$2f$top$2d$level$2d$await$2f$input$2f$db$2d$connection$2e$js__$5b$test$5d$__$28$ecmascript$29$__] = __turbopack_async_dependencies__.then ? (await __turbopack_async_dependencies__)() : __turbopack_async_dependencies__;
"__TURBOPACK__ecmascript__hoisting__location__";
;
const createUser = async (name)=>{
    const command = `CREATE USER ${name}`;
    // This is a normal await, because it's in an async function
    await (0, __TURBOPACK__imported__module__$5b$project$5d2f$crates$2f$turbopack$2d$tests$2f$tests$2f$snapshot$2f$basic$2f$top$2d$level$2d$await$2f$input$2f$db$2d$connection$2e$js__$5b$test$5d$__$28$ecmascript$29$__["dbCall"])({
        command
    });
};
__turbopack_async_result__();
} catch(e) { __turbopack_async_result__(e); } }, false);
})()),
}]);

//# sourceMappingURL=crates_turbopack-tests_tests_snapshot_basic_top-level-await_input_e71653._.js.map