'use client';

import { callPath } from '../lib/http/typed';
import { request } from '../lib/http/client';
import { usePathSWR } from '../lib/http/hooks';
import { itemsPath, ROUTES } from '../lib/http/routes';

export function Items({ id }: { id: string }) {
  const { data } = usePathSWR('/v1/items/{id}', id ? { id } : null);

  async function load() {
    return callPath('/v1/items/{id}', 'get', { id, expand: true });
  }

  async function remove() {
    await request(`${itemsPath('v1')}/${encodeURIComponent(id)}`, {
      method: 'DELETE',
    });
  }

  async function ping() {
    return request(ROUTES.health);
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
