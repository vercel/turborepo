import useSWR from "swr";
import axios from "axios";

const fetcher = (url) => axios.get(url).then((res) => res.data);

const path =
  "https://api.us-east.tinybird.co/v0/pipes/turborepo_time_saved_ticker.json?token=p.eyJ1IjogIjAzYzA0Y2MyLTM1YTAtNDhhNC05ZTZjLThhMWE0NGNhNjhkZiIsICJpZCI6ICJmOWIzMTU5Yi0wOTVjLTQyM2UtOWIwNS04ZDZlNzIyNjEwNzIifQ.A3TOPdm3Lhmn-1x5m6jNvulCQbbgUeQfAIO3IaaAt5k";

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

export default function useTurborepoMinutesSaved():
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
