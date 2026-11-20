// `import()` converts its argument to a string: a URL object, an array with one element
// and an object with `toString` all load a module.
export function load(spec) {
  return import(spec).catch(() => null);
}
