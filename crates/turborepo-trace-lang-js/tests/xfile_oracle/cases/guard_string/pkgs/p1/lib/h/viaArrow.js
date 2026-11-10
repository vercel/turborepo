import { isString, arrow, loose, negated, other, rebound } from '../is';
const get = (spec) => import(spec).catch(() => null);
export function viaArrow(spec) {
  if (arrow(spec)) return import(spec).catch(() => null);
  return null;
}
