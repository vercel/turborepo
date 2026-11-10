import React, { useState } from 'react';
import type { Props } from './props';
import styles from './styles.module.css';

const identity = <T,>(value: T): T => value;
const bounded = <T extends object>(value: T) => value;
const constParam = <const T,>(value: T) => value;

export function Component<T>({ items }: Props<T>) {
  const [open, setOpen] = useState(false);
  const Lazy = React.lazy(() => import('./lazy-panel'));
  return (
    <div className={styles.root} data-text="import x from './not-a-ref'">
      {/* require('./not-a-ref-comment') */}
      it's a "text" with `backticks` and require('./not-a-ref-text')
      {items.map((item) => (
        <Item key={item.id} {...item} onClick={() => setOpen(!open)} />
      ))}
      <T extends="attr">generic-looking tag</T>
      <>
        {open && <Lazy load={require('./in-attribute')} />}
      </>
    </div>
  );
}
