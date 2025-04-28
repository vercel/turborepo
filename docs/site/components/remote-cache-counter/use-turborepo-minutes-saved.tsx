import useSWR from "swr";
import type { TurborepoMinutesSaved } from "#app/api/remote-cache-minutes-saved/route.ts";

export function useTurborepoMinutesSaved(): TurborepoMinutesSaved | undefined {
  const swr = useSWR<TurborepoMinutesSaved, unknown>(
    "/api/remote-cache-minutes-saved",
    () =>
      fetch("/api/remote-cache-minutes-saved").then(
        (res) => res.json() as unknown as TurborepoMinutesSaved
      ),
    {
      revalidateOnMount: true,
      revalidateIfStale: true,
      refreshInterval: 10000,
    }
  );

  return swr.data;
}
