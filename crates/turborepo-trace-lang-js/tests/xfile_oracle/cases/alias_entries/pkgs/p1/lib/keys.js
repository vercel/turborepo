// The same with a loop over the keys.
const T = {
  a: '@shared/routes/a',
  b: '@shared/routes/b',
};
export function all() {
  const out = [];
  for (const k in T) out.push(require(T[k]));
  return out;
}
