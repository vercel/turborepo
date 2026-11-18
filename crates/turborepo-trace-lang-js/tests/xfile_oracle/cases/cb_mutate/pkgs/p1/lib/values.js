// The elements come from a builtin that lists what an object holds.
export function values(table, fix) {
  Object.values(table).forEach(fix);
  return require(table.x.name);
}
