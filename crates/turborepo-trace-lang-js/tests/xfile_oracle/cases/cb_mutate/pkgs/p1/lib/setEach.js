export function setEach(list, fix) {
  new Set(list).forEach(fix);
  return require(list[0].name);
}
