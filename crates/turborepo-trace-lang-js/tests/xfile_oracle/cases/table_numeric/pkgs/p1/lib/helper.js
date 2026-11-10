// A table that is an array, read with a number from the caller.
const T = ['./mods/a', './mods/b', './mods/c'];
export function load(i) {
  return require(T[i]);
}
