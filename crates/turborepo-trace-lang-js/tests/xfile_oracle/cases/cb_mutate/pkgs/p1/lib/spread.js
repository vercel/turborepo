export function spread(list, fix) {
  [...list].forEach(fix);
  return require(list[0].name);
}
