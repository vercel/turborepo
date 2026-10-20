"use client";

import { useEffect, useState } from "react";

export interface TurborepoMinutesSaved {
  total: number;
}

export function useTurborepoMinutesSaved(): TurborepoMinutesSaved | undefined {
  const [data, setData] = useState<TurborepoMinutesSaved | undefined>(
    undefined
  );

  useEffect(() => {
    const fetchData = async () => {
      try {
        const res = await fetch("/api/remote-cache-minutes-saved");
        const json = (await res.json()) as TurborepoMinutesSaved;
        setData(json);
      } catch (error) {
        console.error("Failed to fetch minutes saved:", error);
      }
    };

    // Initial fetch
    void fetchData();

    // Refresh every 10 seconds
    const interval = setInterval(() => {
      void fetchData();
    }, 10000);

    return () => clearInterval(interval);
  }, []);

  return data;
}
