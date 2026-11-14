// The elements are new objects that hold what the caller passed.
export function wrap(spec, fix) {
  [{ inner: spec }].map(fix);
  return require(spec.name);
}
