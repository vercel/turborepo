"use client";

import { useEffect, useState } from "react";

import type { FactoryImagePointer } from "../agent/lib/factory-image-types";
import { FactorySandboxCommand } from "./factory-sandbox-command";

export function LatestFactorySandboxCommand({
  initialSnapshotId
}: {
  readonly initialSnapshotId: string | null;
}) {
  const [snapshotId, setSnapshotId] = useState(initialSnapshotId);

  useEffect(() => {
    let controller: AbortController | null = null;
    async function refresh() {
      if (controller || document.visibilityState !== "visible") return;
      controller = new AbortController();
      try {
        const response = await fetch("/api/factory-image/current", {
          cache: "no-store",
          signal: controller.signal
        });
        if (!response.ok) return;
        const view = (await response.json()) as {
          pointer: FactoryImagePointer | null;
        };
        setSnapshotId(view.pointer?.snapshotId ?? null);
      } catch {
        // Keep the last published command when a refresh is unavailable.
      } finally {
        controller = null;
      }
    }
    void refresh();
    const timer = window.setInterval(() => void refresh(), 15_000);
    return () => {
      controller?.abort();
      window.clearInterval(timer);
    };
  }, []);

  return <FactorySandboxCommand snapshotId={snapshotId} />;
}
