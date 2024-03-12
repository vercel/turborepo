import { unstable_cache } from "next/cache";

export interface QueryResponse {
  meta: { name: string; type: string }[];
  data: {
    last_update_time: string;
    remote_cache_minutes_saved: number;
    local_cache_minutes_saved: number;
  }[];
  rows: number;
  statistics: {
    elapsed: number;
    rows_read: number;
    bytes_read: number;
  };
}

export const REMOTE_CACHE_MINUTES_SAVED_URL = `https://api.us-east.tinybird.co/v0/pipes/turborepo_time_saved_ticker.json?token=${process.env.NEXT_PUBLIC_TINYBIRD_TIME_SAVED_PUBLIC_TOKEN}`;

export const REMOTE_CACHE_METRIC_TAG = "remote-cache-minutes-saved";

export const fetchTimeSaved = async (url: string) => {
  const response = await fetch(url, {
    next: { tags: [REMOTE_CACHE_METRIC_TAG] },
  });
  const data = (await response.json()) as unknown as QueryResponse;
  return data;
};

export const remoteCacheTimeSavedQuery = unstable_cache(
  async (url: string) => fetchTimeSaved(url),
  [REMOTE_CACHE_METRIC_TAG],
  { revalidate: 5 }
);

export const computeTimeSaved = (metrics: QueryResponse): number => {
  const data = metrics.data[0];

  const timeSaved =
    (data.local_cache_minutes_saved + data.remote_cache_minutes_saved) / 60;

  return timeSaved;
};
