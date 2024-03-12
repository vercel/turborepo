"use client";

import { REMOTE_CACHE_MINUTES_SAVED_KEY } from "./data";
import { SWRConfig } from "swr";

export const SwrProvider = ({
  children,
  startingNumber,
}: {
  children: React.ReactNode;
  startingNumber: number;
}) => {
  return (
    <SWRConfig
      value={{
        fallback: {
          [REMOTE_CACHE_MINUTES_SAVED_KEY]: startingNumber,
        },
      }}
    >
      {children}
    </SWRConfig>
  );
};
