(globalThis.TURBOPACK = globalThis.TURBOPACK || []).push(["output/crates_turbopack-tests_tests_snapshot_comptime_not-sure_input_c3b86f._.js", {

"[project]/crates/turbopack-tests/tests/snapshot/comptime/not-sure/input/module.js [test] (ecmascript)": (function({ r: __turbopack_require__, f: __turbopack_module_context__, i: __turbopack_import__, s: __turbopack_esm__, v: __turbopack_export_value__, n: __turbopack_export_namespace__, c: __turbopack_cache__, M: __turbopack_modules__, l: __turbopack_load__, j: __turbopack_dynamic__, P: __turbopack_resolve_absolute_path__, U: __turbopack_relative_url__, R: __turbopack_resolve_module_id_path__, g: global, __dirname, m: module, e: exports, t: require }) { !function() {

module.exports = {};

}.call(this) }),
"[project]/crates/turbopack-tests/tests/snapshot/comptime/not-sure/input/index.ts [test] (ecmascript)": (({ r: __turbopack_require__, f: __turbopack_module_context__, i: __turbopack_import__, s: __turbopack_esm__, v: __turbopack_export_value__, n: __turbopack_export_namespace__, c: __turbopack_cache__, M: __turbopack_modules__, l: __turbopack_load__, j: __turbopack_dynamic__, P: __turbopack_resolve_absolute_path__, U: __turbopack_relative_url__, R: __turbopack_resolve_module_id_path__, g: global, __dirname }) => (() => {
"use strict";

__turbopack_esm__({
    "BubbledError": ()=>BubbledError,
    "isBubbledError": ()=>isBubbledError
});
var __TURBOPACK__imported__module__$5b$project$5d2f$crates$2f$turbopack$2d$tests$2f$tests$2f$snapshot$2f$comptime$2f$not$2d$sure$2f$input$2f$module$2e$js__$5b$test$5d$__$28$ecmascript$29$__ = __turbopack_import__("[project]/crates/turbopack-tests/tests/snapshot/comptime/not-sure/input/module.js [test] (ecmascript)");
"__TURBOPACK__ecmascript__hoisting__location__";
import type { FetchEventResult } from "./module";
import type { TextMapSetter } from "./module";
import type { SpanTypes } from "./module";
;
import type { ContextAPI, Span, SpanOptions, Tracer, AttributeValue, TextMapGetter } from "./module";
let api: typeof import("./module");
// we want to allow users to use their own version of @opentelemetry/api if they
// want to, so we try to require it first, and if it fails we fall back to the
// version that is bundled with Next.js
// this is because @opentelemetry/api has to be synced with the version of
// @opentelemetry/tracing that is used, and we don't want to force users to use
// the version that is bundled with Next.js.
// the API is ~stable, so this should be fine
if (process.env.NEXT_RUNTIME === "edge") {
    api = __turbopack_require__("[project]/crates/turbopack-tests/tests/snapshot/comptime/not-sure/input/module.js [test] (ecmascript)");
} else {
    try {
        api = __turbopack_require__("[project]/crates/turbopack-tests/tests/snapshot/comptime/not-sure/input/module.js [test] (ecmascript)");
    } catch (err) {
        api = __turbopack_require__("[project]/crates/turbopack-tests/tests/snapshot/comptime/not-sure/input/module.js [test] (ecmascript)");
    }
}
const { context, propagation, trace, SpanStatusCode, SpanKind, ROOT_CONTEXT } = api;
const isPromise = <T>(p: any): p is Promise<T> =>{
    return p !== null && typeof p === "object" && typeof p.then === "function";
};
class BubbledError extends Error {
    constructor(public readonly bubble?: boolean, public readonly result?: FetchEventResult){
        super();
    }
}
function isBubbledError(error: unknown): error is BubbledError {
    if (typeof error !== "object" || error === null) return false;
    return error instanceof BubbledError;
}
const closeSpanWithError = (span: Span, error?: Error)=>{
    if (isBubbledError(error) && error.bubble) {
        span.setAttribute("next.bubble", true);
    } else {
        if (error) {
            span.recordException(error);
        }
        span.setStatus({
            code: SpanStatusCode.ERROR,
            message: error?.message
        });
    }
    span.end();
};
type TracerSpanOptions = Omit<SpanOptions, "attributes"> & {
    parentSpan?: Span;
    spanName?: string;
    attributes?: Partial<Record<AttributeNames, AttributeValue | undefined>>;
    hideSpan?: boolean;
};
interface NextTracer {
    getContext(): ContextAPI;
    /**
   * Instruments a function by automatically creating a span activated on its
   * scope.
   *
   * The span will automatically be finished when one of these conditions is
   * met:
   *
   * * The function returns a promise, in which case the span will finish when
   * the promise is resolved or rejected.
   * * The function takes a callback as its second parameter, in which case the
   * span will finish when that callback is called.
   * * The function doesn't accept a callback and doesn't return a promise, in
   * which case the span will finish at the end of the function execution.
   *
   */ trace<T>(type: SpanTypes, fn: (span?: Span, done?: (error?: Error) => any) => Promise<T>): Promise<T>;
    trace<T>(type: SpanTypes, fn: (span?: Span, done?: (error?: Error) => any) => T): T;
    trace<T>(type: SpanTypes, options: TracerSpanOptions, fn: (span?: Span, done?: (error?: Error) => any) => Promise<T>): Promise<T>;
    trace<T>(type: SpanTypes, options: TracerSpanOptions, fn: (span?: Span, done?: (error?: Error) => any) => T): T;
    /**
   * Wrap a function to automatically create a span activated on its
   * scope when it's called.
   *
   * The span will automatically be finished when one of these conditions is
   * met:
   *
   * * The function returns a promise, in which case the span will finish when
   * the promise is resolved or rejected.
   * * The function takes a callback as its last parameter, in which case the
   * span will finish when that callback is called.
   * * The function doesn't accept a callback and doesn't return a promise, in
   * which case the span will finish at the end of the function execution.
   */ wrap<T = (...args: Array<any>) => any>(type: SpanTypes, fn: T): T;
    wrap<T = (...args: Array<any>) => any>(type: SpanTypes, options: TracerSpanOptions, fn: T): T;
    wrap<T = (...args: Array<any>) => any>(type: SpanTypes, options: (...args: any[]) => TracerSpanOptions, fn: T): T;
    /**
   * Starts and returns a new Span representing a logical unit of work.
   *
   * This method do NOT modify the current Context by default. In result, any inner span will not
   * automatically set its parent context to the span created by this method unless manually activate
   * context via `tracer.getContext().with`. `trace`, or `wrap` is generally recommended as it gracefully
   * handles context activation. (ref: https://github.com/open-telemetry/opentelemetry-js/issues/1923)
   */ startSpan(type: SpanTypes): Span;
    startSpan(type: SpanTypes, options: TracerSpanOptions): Span;
    /**
   * Returns currently activated span if current context is in the scope of the span.
   * Returns undefined otherwise.
   */ getActiveScopeSpan(): Span | undefined;
    /**
   * Returns trace propagation data for the currently active context. The format is equal to data provided
   * through the OpenTelemetry propagator API.
   */ getTracePropagationData(): ClientTraceDataEntry[];
}
type NextAttributeNames = "next.route" | "next.page" | "next.rsc" | "next.segment" | "next.span_name" | "next.span_type" | "next.clientComponentLoadCount";
type OTELAttributeNames = `http.${string}` | `net.${string}`;
type AttributeNames = NextAttributeNames | OTELAttributeNames;
/** we use this map to propagate attributes from nested spans to the top span */ const rootSpanAttributesStore = new Map<number, Map<AttributeNames, AttributeValue | undefined>>();
const rootSpanIdKey = api.createContextKey("next.rootSpanId");
let lastSpanId = 0;
const getSpanId = ()=>lastSpanId++;
interface ClientTraceDataEntry {
    key: string;
    value: string;
}
const clientTraceDataSetter: TextMapSetter<ClientTraceDataEntry[]> = {
    set (carrier, key, value) {
        carrier.push({
            key,
            value
        });
    }
};
class NextTracerImpl implements NextTracer {
    /**
   * Returns an instance to the trace with configured name.
   * Since wrap / trace can be defined in any place prior to actual trace subscriber initialization,
   * This should be lazily evaluated.
   */ private getTracerInstance(): Tracer {
        return trace.getTracer("next.js", "0.0.1");
    }
}

})()),
"[project]/crates/turbopack-tests/tests/snapshot/comptime/not-sure/input/index.js [test] (ecmascript)": (function({ r: __turbopack_require__, f: __turbopack_module_context__, i: __turbopack_import__, s: __turbopack_esm__, v: __turbopack_export_value__, n: __turbopack_export_namespace__, c: __turbopack_cache__, M: __turbopack_modules__, l: __turbopack_load__, j: __turbopack_dynamic__, P: __turbopack_resolve_absolute_path__, U: __turbopack_relative_url__, R: __turbopack_resolve_module_id_path__, g: global, __dirname, m: module, e: exports, t: require }) { !function() {

__turbopack_esm__({});
var __TURBOPACK__imported__module__$5b$project$5d2f$crates$2f$turbopack$2d$tests$2f$tests$2f$snapshot$2f$comptime$2f$not$2d$sure$2f$input$2f$index$2e$ts__$5b$test$5d$__$28$ecmascript$29$__ = __turbopack_import__("[project]/crates/turbopack-tests/tests/snapshot/comptime/not-sure/input/index.ts [test] (ecmascript)");
"__TURBOPACK__ecmascript__hoisting__location__";
;

}.call(this) }),
}]);

//# sourceMappingURL=crates_turbopack-tests_tests_snapshot_comptime_not-sure_input_c3b86f._.js.map