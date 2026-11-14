// The load inside the branch the test guards.
export function branch(spec) {
  if (typeof spec === 'string' && spec.startsWith('../area/')) {
    return import(spec).catch(() => null);
  }
  return null;
}
