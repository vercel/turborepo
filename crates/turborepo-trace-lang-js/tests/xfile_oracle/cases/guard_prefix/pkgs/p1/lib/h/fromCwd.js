// A pattern below the working directory: nothing matches it from the repository root.
import path from 'node:path';
export function fromCwd(name) {
  return import(path.join(process.cwd(), 'lib', 'area', name)).catch(() => null);
}
