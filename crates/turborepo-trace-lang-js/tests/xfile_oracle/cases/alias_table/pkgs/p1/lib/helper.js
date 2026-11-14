// The table's values are aliases of the jest configuration (moduleNameMapper).
const T = {
  a: '@shared/routes/a',
  b: '@shared/routes/b',
};
export function load(key) {
  return require(T[key]);
}
