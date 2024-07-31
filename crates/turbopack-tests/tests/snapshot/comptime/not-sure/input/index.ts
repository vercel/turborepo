import type { FetchEventResult } from "./module";
import type { TextMapSetter } from "./module";
import type { SpanTypes } from "./module";
import { LogSpanAllowList, NextVanillaSpanAllowlist } from "./module";

import type {
  ContextAPI,
  Span,
  SpanOptions,
  Tracer,
  AttributeValue,
  TextMapGetter,
} from "./module";

let api: typeof import("./module");

// we want to allow users to use their own version of @opentelemetry/api if they
// want to, so we try to require it first, and if it fails we fall back to the
// version that is bundled with Next.js
// this is because @opentelemetry/api has to be synced with the version of
// @opentelemetry/tracing that is used, and we don't want to force users to use
// the version that is bundled with Next.js.
// the API is ~stable, so this should be fine
if (process.env.NEXT_RUNTIME === "edge") {
  api = require("./module");
} else {
  try {
    api = require("./module");
  } catch (err) {
    api = require("./module");
  }
}

const { context, propagation, trace, SpanStatusCode, SpanKind, ROOT_CONTEXT } =
  api;

const isPromise = <T>(p: any): p is Promise<T> => {
  return p !== null && typeof p === "object" && typeof p.then === "function";
};

export class BubbledError extends Error {
  constructor(
    public readonly bubble?: boolean,
    public readonly result?: FetchEventResult
  ) {
    super();
  }
}

export function isBubbledError(error: unknown): error is BubbledError {
  if (typeof error !== "object" || error === null) return false;
  return error instanceof BubbledError;
}

const closeSpanWithError = (span: Span, error?: Error) => {
  if (isBubbledError(error) && error.bubble) {
    span.setAttribute("next.bubble", true);
  } else {
    if (error) {
      span.recordException(error);
    }
    span.setStatus({ code: SpanStatusCode.ERROR, message: error?.message });
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
   */
  trace<T>(
    type: SpanTypes,
    fn: (span?: Span, done?: (error?: Error) => any) => Promise<T>
  ): Promise<T>;
  trace<T>(
    type: SpanTypes,
    fn: (span?: Span, done?: (error?: Error) => any) => T
  ): T;
  trace<T>(
    type: SpanTypes,
    options: TracerSpanOptions,
    fn: (span?: Span, done?: (error?: Error) => any) => Promise<T>
  ): Promise<T>;
  trace<T>(
    type: SpanTypes,
    options: TracerSpanOptions,
    fn: (span?: Span, done?: (error?: Error) => any) => T
  ): T;

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
   */
  wrap<T = (...args: Array<any>) => any>(type: SpanTypes, fn: T): T;
  wrap<T = (...args: Array<any>) => any>(
    type: SpanTypes,
    options: TracerSpanOptions,
    fn: T
  ): T;
  wrap<T = (...args: Array<any>) => any>(
    type: SpanTypes,
    options: (...args: any[]) => TracerSpanOptions,
    fn: T
  ): T;

  /**
   * Starts and returns a new Span representing a logical unit of work.
   *
   * This method do NOT modify the current Context by default. In result, any inner span will not
   * automatically set its parent context to the span created by this method unless manually activate
   * context via `tracer.getContext().with`. `trace`, or `wrap` is generally recommended as it gracefully
   * handles context activation. (ref: https://github.com/open-telemetry/opentelemetry-js/issues/1923)
   */
  startSpan(type: SpanTypes): Span;
  startSpan(type: SpanTypes, options: TracerSpanOptions): Span;

  /**
   * Returns currently activated span if current context is in the scope of the span.
   * Returns undefined otherwise.
   */
  getActiveScopeSpan(): Span | undefined;

  /**
   * Returns trace propagation data for the currently active context. The format is equal to data provided
   * through the OpenTelemetry propagator API.
   */
  getTracePropagationData(): ClientTraceDataEntry[];
}

type NextAttributeNames =
  | "next.route"
  | "next.page"
  | "next.rsc"
  | "next.segment"
  | "next.span_name"
  | "next.span_type"
  | "next.clientComponentLoadCount";
type OTELAttributeNames = `http.${string}` | `net.${string}`;
type AttributeNames = NextAttributeNames | OTELAttributeNames;

/** we use this map to propagate attributes from nested spans to the top span */
const rootSpanAttributesStore = new Map<
  number,
  Map<AttributeNames, AttributeValue | undefined>
>();
const rootSpanIdKey = api.createContextKey("next.rootSpanId");
let lastSpanId = 0;
const getSpanId = () => lastSpanId++;

export interface ClientTraceDataEntry {
  key: string;
  value: string;
}

const clientTraceDataSetter: TextMapSetter<ClientTraceDataEntry[]> = {
  set(carrier, key, value) {
    carrier.push({
      key,
      value,
    });
  },
};

class NextTracerImpl implements NextTracer {
  /**
   * Returns an instance to the trace with configured name.
   * Since wrap / trace can be defined in any place prior to actual trace subscriber initialization,
   * This should be lazily evaluated.
   */
  private getTracerInstance(): Tracer {
    return trace.getTracer("next.js", "0.0.1");
  }
}
