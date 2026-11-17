// One jest configuration at the root: the working directory of every test is the root.
import path from 'path';
export function mount() {
  const m = require(path.join(process.cwd(), 'config/routes'));
  for (const r of m.routes) require(path.join(process.cwd(), 'config', r.handler));
  return m;
}
