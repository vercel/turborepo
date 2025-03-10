export const revalidate = 5;

export const pathKey = `https://api.us-east.tinybird.co/v0/pipes/turborepo_time_saved_ticker.json?token=${process.env.TINYBIRD_TIME_SAVED_TOKEN}`;

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

export interface TurborepoMinutesSaved {
  total: number;
  remoteCacheMinutesSaved: number;
  localCacheMinutesSaved: number;
}

export const getRemoteCacheSavedMinutes =
  async (): Promise<TurborepoMinutesSaved> => {
    const raw = await fetch(pathKey).then(
      (res) => res.json() as unknown as QueryResponse
    );

    const data = raw.data[0];

    return {
      total:
        (data?.remote_cache_minutes_saved ?? 0) +
        (data?.local_cache_minutes_saved ?? 0),
      remoteCacheMinutesSaved: data?.remote_cache_minutes_saved ?? 0,
      localCacheMinutesSaved: data?.local_cache_minutes_saved ?? 0,
    };
  };

export const GET = async (): Promise<Response> => {
  return Response.json(await getRemoteCacheSavedMinutes());
};
