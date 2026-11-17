// The callback is the result of calling what the caller passed (a merger factory).
export function merged(spec, make) {
  [{ handler: spec }].map(make(spec));
  return require(spec.name);
}
