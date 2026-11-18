// The names are built at run time: no string of this file names a module.
const n = (c) => './mods/' + String.fromCharCode(c);
export function touch(w) {
  w.inner.name = n(100);
}
export const keep = (next) => (current) => {
  current.seen = next.name;
  return current;
};
export const taint = (next) => (current) => {
  current.handler.name = n(102);
  return current;
};
export const taintNext = (next) => (current) => {
  next.name = n(103);
  return current;
};
