import useSWR from "swr";
import axios from "axios";

const fetcher = (url: string) =>
  axios.get(url).then((res) => res.data as QueryResponse);

const path = `https://api.us-east.tinybird.co/v0/pipes/turborepo_time_saved_ticker.json?token=${process.env.NEXT_PUBLIC_TINYBIRD_TIME_SAVED_PUBLIC_TOKEN}`;

const REFRESH_INTERVAL_IN_MS = 3500;

interface QueryResponse {
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

export function useTurborepoMinutesSaved():
  | {
      last_update_time: string;
      remote_cache_minutes_saved: number;
      local_cache_minutes_saved: number;
    }
  | undefined {
  const swr = useSWR<QueryResponse, unknown>(path, fetcher, {
    refreshInterval: REFRESH_INTERVAL_IN_MS,
  });

  return swr.data?.data[0];
}
