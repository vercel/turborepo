import React from 'react';
import Button from './Button';

export default function App({ user }) {
  const icon = require('./icon.svg');
  return (
    <main title='it&apos;s "quoted"'>
      <h1>Hello, {user.name}! Don't import './nope'</h1>
      <Button icon={icon} onClick={() => import('./analytics').then((m) => m.track())}>
        Click / here /
      </Button>
      {user.admin ? <Admin /> : null}
    </main>
  );
}
