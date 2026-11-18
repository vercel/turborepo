import type { Fetcher, SWRConfiguration } from 'swr';
import useSWR from 'swr';
import { apiFetch } from './fetch-api';
import { buildUrl } from './typed/url';

export function useLazySWR<
  Data,
  ErrorType = Error,
  Key extends string | null = string | null,
>(
  key: Key,
  fetcher?: Fetcher<Data, Key>,
  config?: SWRConfiguration<Data, ErrorType, Fetcher<Data, Key>>,
) {
  return useSWR<Data, ErrorType, Key>(key, fetcher ?? null, {
    suspense: true,
    ...config,
  });
}

export function useTypedSWR<P extends string>(path: P, params: object | null) {
  return useSWR<unknown>(
    () => {
      if (!params) {
        return null;
      }
      try {
        return buildUrl(path, params as unknown as Record<string, unknown>);
      } catch {
        return `__invalid_typed_swr_key__:${path}:${JSON.stringify(params)}`;
      }
    },
    () => {
      if (!params) {
        throw new Error('no params');
      }
      const url = buildUrl(path, params as unknown as Record<string, unknown>);
      return apiFetch(url, {
        method: 'GET',
      });
    },
  );
}
