'use client';

import useSWR from 'swr';

// The URL is a prop: every element of the component is a call with its props.
export function Feed({ endpoint, limit = 10 }: { endpoint: string; limit?: number }) {
  const { data } = useSWR(`${endpoint}?limit=${limit}`, (url: string) =>
    fetch(url).then((r) => r.json()),
  );
  return <pre>{JSON.stringify(data)}</pre>;
}
