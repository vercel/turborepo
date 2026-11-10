// A string method on the whole specifier: nothing of the value is known at a caller.
export function load(name) {
  return require(name.slice(1));
}
