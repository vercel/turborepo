'use client';

import { useLazySWR } from '../lib/api/swr';
import { apiFetch } from '../lib/api/fetch-api';

export function Dash({ slug }: { slug: string }) {
  const { data } = useLazySWR<Record<string, unknown>>(
    `/api/dash?teamId=${slug}`,
    async (endpoint: string) => {
      try {
        const data = await apiFetch<Record<string, unknown>>(endpoint, {
          headers: { accept: 'application/json' },
        });
        return data || {};
      } catch (error) {
        throw error;
      }
    },
    {
      revalidateOnFocus: false,
    },
  );
  return <pre>{JSON.stringify(data)}</pre>;
}
