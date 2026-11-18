import useSWR from 'swr';
import type { Fetcher, SWRConfiguration } from 'swr';
import { request } from './client';
import { fillPath } from './typed/path';

// Suspends until the first response; everything else is forwarded to SWR.
export function useDeferredSWR<T, E = Error, K extends string | null = string | null>(
  key: K,
  load?: Fetcher<T, K>,
  options?: SWRConfiguration<T, E, Fetcher<T, K>>,
) {
  return useSWR<T, E, K>(key, load ?? null, { ...options, suspense: true });
}

export function usePathSWR<T extends string>(template: T, values: object | null) {
  return useSWR<unknown>(
    () => {
      if (values === null) return null;
      try {
        return fillPath(template, values as unknown as Record<string, unknown>);
      } catch {
        return `unfillable:${template}`;
      }
    },
    () => {
      if (values === null) {
        throw new Error(`${template}: missing values`);
      }
      return request(fillPath(template, values as unknown as Record<string, unknown>), {
        method: 'GET',
      });
    },
  );
}
