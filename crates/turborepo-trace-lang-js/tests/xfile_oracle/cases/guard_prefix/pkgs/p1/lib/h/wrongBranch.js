// Only looks like a guard: the load is where the test failed.
export function wrongBranch(spec) {
  if (spec.startsWith('../area/')) return null;
  return import(spec).catch(() => null);
}
