export function load(opts) {
  return require(opts.name);
}
export function loadAll(list) {
  return list.map((f) => require(f));
}
