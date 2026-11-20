import { add } from './add.js';

export function mul(a, b) {
  let out = 0;
  for (let i = 0; i < b; i++) out = add(out, a);
  return out;
}
