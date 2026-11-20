// An array method hands the elements to a callback the caller supplies: the callback can
// change them before the helper reads them.
export function each(list, fix) {
  list.forEach(fix);
  return require(list[0].name);
}
