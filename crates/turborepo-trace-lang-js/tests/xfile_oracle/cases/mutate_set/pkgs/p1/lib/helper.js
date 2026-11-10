// The helper fills in a missing option before the load.
export function load(opts) {
  if (!opts.name) opts.name = './mods/c';
  return require(opts.name);
}
