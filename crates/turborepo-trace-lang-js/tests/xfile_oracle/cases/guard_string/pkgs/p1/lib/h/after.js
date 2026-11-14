import { isString, arrow, loose, negated, other, rebound } from '../is';
const get = (spec) => import(spec).catch(() => null);
export function after(spec) {
  if (isString(spec)) get('../mods/q.js');
  return import(spec).catch(() => null);
}
