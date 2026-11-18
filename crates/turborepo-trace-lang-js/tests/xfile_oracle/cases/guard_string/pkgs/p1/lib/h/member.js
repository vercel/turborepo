import { isString, arrow, loose, negated, other, rebound } from '../is';
const get = (spec) => import(spec).catch(() => null);
export function member(opts) {
  if (isString(opts.spec)) {
    opts.swap();
    return import(opts.spec).catch(() => null);
  }
  return null;
}
