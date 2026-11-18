// An optional second parameter that one caller leaves out.
export function load(name, ext) {
  return require(name + (ext || ''));
}
