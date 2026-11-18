'use client';

import { useDeferredSWR } from '../lib/http/hooks';
import { useTeam } from '../lib/use-team';

export function membersKey(teamId: string): string {
  return `/api/teams/${teamId}/members?limit=20`;
}

export function Members() {
  const { team } = useTeam();

  const { data } = useDeferredSWR<{ members: string[] }>(
    team ? membersKey(team.id) : null,
    async (url: string) => {
      const response = await fetch(url, {
        headers: { 'Content-Type': 'application/json' },
      });
      if (!response.ok) {
        throw new Error(`members: ${response.status}`);
      }
      return response.json();
    },
  );
  return <ul>{data?.members.length}</ul>;
}
