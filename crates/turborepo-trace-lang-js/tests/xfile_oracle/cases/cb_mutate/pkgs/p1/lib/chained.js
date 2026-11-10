export function chained(list, fix) {
  list.filter(Boolean).map(fix);
  return require(list[0].name);
}
