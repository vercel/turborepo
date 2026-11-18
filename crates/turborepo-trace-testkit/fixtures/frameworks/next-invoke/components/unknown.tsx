'use client';

import { request } from '../lib/http/client';

declare function computeUrl(kind: string): string;

export function Unknown(props: { kind: string }) {
  async function load() {
    const a = await fetch(localStorage.getItem('endpoint')!);
    const b = await request(computeUrl(props.kind));
    return [a, b];
  }
  return <button onClick={load}>load</button>;
}
