import { lazy, useState } from 'react';
import type { Row } from './row';

// A generic function type after a colon and in an alias: type parameters, not JSX.
interface Store {
  sync: <T>(refresh: () => Promise<T>) => Promise<T>;
}
type Handle = {
  run?: <T>(
    fn: (signal?: AbortSignal) => Promise<T>,
    options?: { retries: number },
  ) => Promise<Result<T>>;
};
type Unbox = <T>(value: Box<T> | T) => T;
const toValue: <T>(value: Box<T> | T) => T = (value) => require('./unbox')(value);

// A type parameter with a default.
export const useLast = <A = any,>(value: A) => useState(value);

// Type arguments on elements.
export function Table<K extends keyof Row>(props: Props<Row, K>) {
  const Lazy = lazy(() => import('./lazy-cell'));
  return (
    <Grid<Row, K>
      {...(props as Parameters<typeof Grid<Row, K>>[0])}
      render={(row) => <Cell<Row[K]> value={row} />}
    >
      <Filter<Filters['status']> name="status" load={require('./in-attribute')} />
      <Select<Option, true> multi>
        (optional) {import('./in-child')}
      </Select>
      <Field<() => void> on={<Hint<'a>' | `b>${string}`> />} />
      <Lazy />
    </Grid>
  );
}

export const after = require('./after-the-elements');
