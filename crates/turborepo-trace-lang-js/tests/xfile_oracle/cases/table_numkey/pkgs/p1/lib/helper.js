// A table with numeric keys, read with numbers from the callers.
const T = {
  1: './mods/a',
  2: './mods/b',
  3: './mods/c',
};
export function load(key) {
  return require(T[key]);
}
export function byId(opts) {
  return require(T[opts.id]);
}
