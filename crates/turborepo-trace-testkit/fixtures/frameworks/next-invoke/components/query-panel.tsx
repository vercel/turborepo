'use client';

import { useJobsQuery, useHealth, useDeleteItem } from '../lib/use-jobs-query';

export function QueryPanel() {
  const jobs = useJobsQuery('t1');
  const health = useHealth();
  const del = useDeleteItem();
  return (
    <button onClick={() => del.mutate('i1')}>
      {String(jobs.data)} {String(health.data)}
    </button>
  );
}
