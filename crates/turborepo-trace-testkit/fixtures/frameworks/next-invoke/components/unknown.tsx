'use client';

import { apiFetch } from '../lib/api/fetch-api';

declare function computeUrl(kind: string): string;

export function Unknown(props: { kind: string }) {
  async function load() {
    const a = await fetch(localStorage.getItem('endpoint')!);
    const b = await apiFetch(computeUrl(props.kind));
    return [a, b];
  }
  return <button onClick={load}>load</button>;
}
