// The helper exchanges two properties of the argument before the load.
export function load(opts) {
  const t = opts.first;
  opts.first = opts.second;
  opts.second = t;
  return require(opts.first);
}
