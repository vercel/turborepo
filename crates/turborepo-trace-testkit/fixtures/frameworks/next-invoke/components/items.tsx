'use client';

import { typedFetch } from '../lib/api/typed';
import { apiFetch } from '../lib/api/fetch-api';
import { useTypedSWR } from '../lib/api/swr';
import { API_ITEMS_VERSION, API_ROUTES } from '../lib/api/endpoints';

export function Items({ id }: { id: string }) {
  const { data } = useTypedSWR('/v1/items/{id}', id ? { id } : null);

  async function load() {
    return typedFetch('/v1/items/{id}', 'get', { id, expand: true });
  }

  async function remove() {
    await apiFetch(`${API_ITEMS_VERSION('v1')}/${encodeURIComponent(id)}`, {
      method: 'DELETE',
    });
  }

  async function ping() {
    return apiFetch(API_ROUTES.HEALTH);
  }

  return (
    <div>
      <button onClick={load}>load</button>
      <button onClick={remove}>remove</button>
      <button onClick={ping}>ping</button>
      {String(data)}
    </div>
  );
}
