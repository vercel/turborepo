// A prefix test that throws.
export function prefix(spec) {
  if (!spec.startsWith('../area/')) throw new Error('outside');
  return import(spec).catch(() => null);
}
