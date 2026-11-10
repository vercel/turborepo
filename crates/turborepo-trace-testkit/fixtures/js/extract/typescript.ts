/// <reference path="./globals.d.ts" />
/// <reference path="relative-without-dot.d.ts" />
/// <reference types="node" />
/// <reference lib="es2022" />
import type { Config } from './config';
import type * as Types from './types';
import type DefaultType from './default-type';
import { type Inline, value } from './inline-type';
import { type OnlyInlineTypes } from './only-inline';
import type from './default-named-type';
import fs = require('fs');
import type TypeEq = require('./type-eq');
export import Reexported = require('./export-import');
import Alias = Namespace.Inner;
export type { Exported } from './exported-types';
export type * from './exported-star-types';
export { type Mixed, runtime } from './mixed-export';

declare module 'ambient-module' {
  export const notARef: string;
}

interface NodeRequireLike {
  require(id: string): unknown;
  resolve(id?: string): string;
}

export function load<T extends object>(input: T): Promise<T> {
  const ratio = input! / 2 / 3;
  const asserted = <any>require('./type-assertion');
  const generic = (<T,>(x: T) => x)(input);
  return import('./lazy-module');
}

export async function typeQuery(): Promise<typeof import('./type-query')> {
  return (await import('./runtime-dynamic')).default;
}

enum Color { Red = 'import("./not-a-ref")' }
abstract class Base<T> { protected abstract run?(): void; }
const nonNull = map.get('k')!.value / 2;
const optional = config?.require?.('./not-a-ref');
