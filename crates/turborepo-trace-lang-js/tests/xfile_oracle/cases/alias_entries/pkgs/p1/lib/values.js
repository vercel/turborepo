// The same with Object.values.
const T = {
  a: '@shared/routes/a',
  b: '@shared/routes/b',
};
export function all() {
  return Object.values(T).map((f) => require(f));
}
