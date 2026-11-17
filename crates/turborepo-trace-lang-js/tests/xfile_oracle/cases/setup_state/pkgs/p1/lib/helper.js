import { getDir } from './config';
export function load(name) {
  return require(getDir() + '/' + name);
}
