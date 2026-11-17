// A workspace package name and a path inside it, from the caller.
export function load(pkg, sub) {
  try {
    return require(pkg + '/' + sub);
  } catch {
    return null;
  }
}
