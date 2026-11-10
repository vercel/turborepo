// An array at the caller, joined by the helper.
export function load(parts) {
  return require(parts.join('/'));
}
