'use client';

import { useMemo } from 'react';
import { useDeferredSWR } from '../lib/http/hooks';
import { ROUTES } from '../lib/http/routes';

async function loadJobs(key: string): Promise<{ jobs: string[] }> {
  return (await fetch(key)).json();
}

export function Jobs({ team, active }: { team: string; active: boolean }) {
  const key = useMemo(() => {
    if (!(active && team)) {
      return null;
    }
    const search = new URLSearchParams({ team, page: '1' });
    return `${ROUTES.jobs}?${search.toString()}`;
  }, [active, team]);

  const { data } = useDeferredSWR<{ jobs: string[] }, Error>(key, loadJobs, {
    refreshInterval: 5000,
  });
  return <ul>{data?.jobs.length}</ul>;
}
