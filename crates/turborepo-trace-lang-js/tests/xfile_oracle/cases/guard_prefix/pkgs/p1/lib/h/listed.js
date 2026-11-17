// Membership in a list of the file.
const ALLOWED = ['../area/b.js', '../area/sub/e.js'];
export function listed(spec) {
  if (!ALLOWED.includes(spec)) return null;
  return import(spec).catch(() => null);
}
