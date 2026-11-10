// The caller resolves a workspace package and passes the file.
export function load(file) {
  return require(file);
}
