// @flow
// Flow generics are not JSX: the heuristic fails and the file is re-read without JSX.
import type { Node } from './types';
import { helper } from './helper';
const identity = <T>(x: T): T => x;
function wrap<T>(x: T): Array<T> { return [x]; }
const lazy = () => import('./lazy');
