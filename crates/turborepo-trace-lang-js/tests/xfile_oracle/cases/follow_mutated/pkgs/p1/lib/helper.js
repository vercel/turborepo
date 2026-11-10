// The helper loads the route module the caller names, then the handlers it lists
// (relative to the helper).
export function mount(file) {
  const m = require(file);
  for (const r of m.routes) require(r.handler);
  return m;
}
