// A string method on the parameter: the value is not known at any caller, so every caller
// has to keep an open record (or the site stays open).
export function load(spec) {
  return require(spec.trim());
}
