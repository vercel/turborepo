export function isString(v) {
  return typeof v === 'string';
}
export const arrow = (v) => typeof v === 'string';
// Not a test for a string: true for some objects too.
export function loose(v) {
  return typeof v === 'string' || !!v.ok;
}
export function negated(v) {
  return typeof v !== 'string';
}
// Tests its second parameter.
export function other(v, w) {
  return typeof w === 'string';
}
// Changes its parameter before the test.
export function rebound(v) {
  v = String(v);
  return typeof v === 'string';
}
