// State of the helper's module that one caller sets.
let current = './mods/a';
export function setName(n) {
  current = n;
}
export function load() {
  return require(current);
}
