import { pick } from './barrel';
export function load(name) {
  return require(pick(name));
}
