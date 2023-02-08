"use client";

import type React from "react";
import { useRouter, usePathname } from "next/dist/client/components/navigation";
import { useEffect } from "react";
import { subscribeToUpdate } from "./hmr-client";
import { ReactDevOverlay } from "./client";

import { initializeHMR } from "@vercel/turbopack-next/dev/client";

type HotReloadProps = React.PropsWithChildren<{
  assetPrefix?: string;
}>;

let initialized = false;

function initializeHMROnce(assetPrefix: string) {
  if (initialized) {
    return;
  }

  initialized = true;
  initializeHMR({
    assetPrefix,
  });
}

export default function HotReload({
  assetPrefix = "",
  children,
}: HotReloadProps) {
  const router = useRouter();
  const path = usePathname()!.slice(1);

  useEffect(() => {
    const unsubscribe = subscribeToUpdate(
      {
        path,
        headers: {
          rsc: "1",
        },
      },
      (update) => {
        if (update.type !== "issues") {
          router.refresh();
        }
      }
    );
    return unsubscribe;
  }, [router, path]);

  useEffect(() => {
    // TODO(alexkirsz) This should handle uninitialize as well. React will
    // re-render this component twice, so right now we need special logic to
    // prevent double-initialization.
    initializeHMROnce(assetPrefix);
  }, []);

  return <ReactDevOverlay globalOverlay={true}>{children}</ReactDevOverlay>;
}
