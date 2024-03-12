import axios from "axios";

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

export const REMOTE_CACHE_MINUTES_SAVED_KEY =
  "https://api.us-east.tinybird.co/v0/pipes/turborepo_time_saved_ticker.json?token=p.eyJ1IjogIjAzYzA0Y2MyLTM1YTAtNDhhNC05ZTZjLThhMWE0NGNhNjhkZiIsICJpZCI6ICJmOWIzMTU5Yi0wOTVjLTQyM2UtOWIwNS04ZDZlNzIyNjEwNzIifQ.A3TOPdm3Lhmn-1x5m6jNvulCQbbgUeQfAIO3IaaAt5k";

const fetcher = (url: string) =>
  axios.get(url).then((res) => res.data as QueryResponse);

export const remoteCacheTimeSavedQuery = () =>
  fetcher(REMOTE_CACHE_MINUTES_SAVED_KEY);

export const computeTimeSaved = (metrics: QueryResponse): number => {
  const data = metrics.data[0];

  const timeSaved =
    (data.local_cache_minutes_saved + data.remote_cache_minutes_saved) / 60;

  return timeSaved;
};
