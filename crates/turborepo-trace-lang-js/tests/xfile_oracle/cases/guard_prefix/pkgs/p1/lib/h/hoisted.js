// Only looks like a guard: the function declaration runs before the test.
export function hoisted(spec) {
  const early = inner();
  if (!spec.startsWith('../area/')) return early;
  return early;
  function inner() {
    return import(spec).catch(() => null);
  }
}
