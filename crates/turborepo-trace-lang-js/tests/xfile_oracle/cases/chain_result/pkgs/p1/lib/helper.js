export function load(spec) {
  return require(spec);
}
export { many } from './many';
// The result holds a function of this module: a caller can call it through the result.
export function api() {
  return { go: load, list: [load] };
}
export function self() {
  return load;
}
export function ns() {
  return require('./helper');
}
