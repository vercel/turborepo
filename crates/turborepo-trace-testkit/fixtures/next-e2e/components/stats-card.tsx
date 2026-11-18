'use client';

import { useEffect, useState } from 'react';

export function StatsCard() {
  const [stats, setStats] = useState<{ visitors: number } | null>(null);
  useEffect(() => {
    fetch('/api/stats')
      .then((r) => r.json())
      .then(setStats);
  }, []);
  return <p data-testid="visitors">{stats ? stats.visitors : '…'}</p>;
}
