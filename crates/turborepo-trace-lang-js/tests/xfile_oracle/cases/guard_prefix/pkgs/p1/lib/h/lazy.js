// A loader made behind the test, called later.
export function lazy(spec) {
  if (!spec.startsWith('../area/')) throw new Error('outside');
  return { load: () => import(spec).catch(() => null) };
}
