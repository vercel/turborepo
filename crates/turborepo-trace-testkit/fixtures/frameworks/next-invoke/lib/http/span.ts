export function withSpan<R>(label: string, body: (span: unknown) => R): R {
  return body(label);
}
