export function entries(table, fix) {
  for (const [, v] of Object.entries(table)) fix(v);
  return require(table.x.name);
}
