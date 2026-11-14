// The key is the length of the caller's array.
const T = ['./mods/a', './mods/b', './mods/c'];
export function load(list) {
  return require(T[list.length]);
}
