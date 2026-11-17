export function trace<T>(fn: (span: unknown) => T, _opts: { name: string }): T {
  return fn(undefined);
}
