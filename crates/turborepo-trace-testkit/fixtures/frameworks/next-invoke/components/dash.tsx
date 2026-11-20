'use client';

import { useDeferredSWR } from '../lib/http/hooks';
import { request } from '../lib/http/client';

export function Dash({ slug }: { slug: string }) {
  const { data } = useDeferredSWR<{ [key: string]: unknown }>(
    `/api/dash?owner=${slug}`,
    async (path: string) => {
      try {
        const body = await request<{ [key: string]: unknown }>(path, {
          headers: { 'x-requested-with': 'dash' },
        });
        return body ?? {};
      } catch (err) {
        console.error(err);
        throw err;
      }
    },
    { revalidateOnFocus: false },
  );
  return <pre>{JSON.stringify(data)}</pre>;
}
