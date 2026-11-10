// The start of a package name is in the helper, the rest comes from the caller: the
// module is a workspace package.
export function load(name) {
  return require('p' + name);
}
