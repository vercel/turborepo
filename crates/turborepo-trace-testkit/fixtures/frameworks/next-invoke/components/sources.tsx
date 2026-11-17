'use client';

import { useState } from 'react';
import { useSearchParams } from 'next/navigation';

import { apiFetch } from '../lib/api/fetch-api';

// The URL comes from the request, although the text of the argument (`next`) does not say so.
export function Next() {
  const params = useSearchParams();
  const next = params.get('next');
  return <button onClick={() => fetch(next!)}>next</button>;
}

// What the user typed: React state set from an input event.
export function Tester() {
  const [target, setTarget] = useState('');
  return (
    <form onSubmit={() => fetch(target)}>
      <input onChange={(e) => setTarget(e.target.value)} />
    </form>
  );
}

// A member named like a request field (`.query`) of a value the code computes.
export function Computed(props: { client: { query: string } }) {
  return <button onClick={() => fetch(props.client.query)}>go</button>;
}

// A wrapper that adds a prefix, called with a path from storage.
export function Last() {
  return <button onClick={() => apiFetch(sessionStorage.getItem('last')!)}>last</button>;
}
