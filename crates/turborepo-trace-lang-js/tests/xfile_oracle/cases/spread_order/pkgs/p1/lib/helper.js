// The module is a property of an options object that callers build with a spread of
// something unknown: written after the spread it is what was written, written before
// it the spread replaces it.
export function load(opts) {
  return require(opts.name);
}
