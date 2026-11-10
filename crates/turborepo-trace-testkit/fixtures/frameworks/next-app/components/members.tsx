'use client';
import useSWR from 'swr';
import { useRouter } from 'next/navigation';
import { API_BASE } from '../lib/constants';

export function Members({ teamId }: { teamId: string }) {
  const router = useRouter();
  const { data } = useSWR(`/api/teams/${teamId}/members`, (u: string) => fetch(u).then((r) => r.json()));
  async function ping(endpoint: string) {
    await fetch('/api/health');
    await fetch(`${API_BASE}/v1/teams`);
    await fetch('/api/v1/projects');
    await fetch(endpoint);
    // Unknown: may call any handler.
    await fetch(window.__ENDPOINT__);
  }
  return (
    <ul className="divide-y" onClick={() => router.push(`/${teamId}/photo/1`)}>
      {data?.map((m) => <li key={m}>{m}</li>)}
    </ul>
  );
}
