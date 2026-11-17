'use client';

import { useMemo } from 'react';
import { useLazySWR } from '../lib/api/swr';
import { API_ROUTES } from '../lib/api/endpoints';

async function fetchJobsPage(url: string): Promise<{ jobs: string[] }> {
  const res = await fetch(url);
  return res.json();
}

export function Jobs({ teamId, enabled }: { teamId: string; enabled: boolean }) {
  const url = useMemo(() => {
    if (!enabled || !teamId) return null;
    const params = new URLSearchParams({ teamId });
    return `${API_ROUTES.JOBS}?${params.toString()}`;
  }, [enabled, teamId]);

  const { data } = useLazySWR<{ jobs: string[] }, Error>(url, fetchJobsPage, {
    refreshInterval: 1000,
  });
  return <ul>{data?.jobs.length}</ul>;
}
