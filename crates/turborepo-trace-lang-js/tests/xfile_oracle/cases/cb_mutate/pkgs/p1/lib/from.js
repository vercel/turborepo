export function from(list, fix) {
  Array.from(list, fix);
  return require(list[0].name);
}
