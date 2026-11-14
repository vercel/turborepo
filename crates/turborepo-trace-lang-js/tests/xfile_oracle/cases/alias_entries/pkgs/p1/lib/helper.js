// The values of a table of aliases leave it through Object.entries.
const T = {
  a: '@shared/routes/a',
  b: '@shared/routes/b',
};
export function except(skip) {
  return Object.entries(T)
    .filter(([k]) => k !== skip)
    .map(([, f]) => require(f));
}
