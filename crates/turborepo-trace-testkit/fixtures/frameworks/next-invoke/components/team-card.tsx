'use client';

import useSWR from 'swr';
import { useTeam } from '../lib/use-team';

interface Activity {
  id: string;
}

export function TeamCard() {
  const { team } = useTeam();
  const since = new Date();

  const { data } = useSWR<{
    activities: Activity[];
  } | null>(
    team?.id
      ? `/api/teams/${team.id}?since=${since.toISOString()}`
      : null,
    async (url: string) => {
      const response = await fetch(url);
      if (!response.ok) {
        return null;
      }
      return response.json();
    },
  );
  return <div>{data?.activities.length}</div>;
}
