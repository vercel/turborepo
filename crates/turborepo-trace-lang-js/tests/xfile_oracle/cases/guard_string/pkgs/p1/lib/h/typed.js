import { isString, arrow, loose, negated, other, rebound } from '../is';
const get = (spec) => import(spec).catch(() => null);
export function typed(spec) {
  if (typeof spec === 'string') return import(spec).catch(() => null);
  return null;
}
