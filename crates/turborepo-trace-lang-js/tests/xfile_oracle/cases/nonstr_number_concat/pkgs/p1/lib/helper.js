// A number from the caller is part of the specifier.
export function load(base, n) {
  return require(base + n);
}
