"use client";

import { InfoIcon } from "lucide-react";
import Link from "next/link";
import { useEffect, useState } from "react";

const PRODUCTION_DOMAIN = "turborepo.dev";

/**
 * Convert subdomain format to display version.
 * Subdomain format: "v2-3-1" -> "2.3.1"
 */
function subdomainToVersion(subdomain: string): string {
  return subdomain.replace(/^v/, "").replace(/-/g, ".");
}

export function VersionWarning() {
  const [version, setVersion] = useState<string | null>(null);

  useEffect(() => {
    const host = window.location.host;

    // Check if we're on a subdomain of turborepo.dev (e.g., v2-3-1.turborepo.dev)
    if (host === PRODUCTION_DOMAIN || !host.endsWith(`.${PRODUCTION_DOMAIN}`)) {
      return;
    }

    // Extract version from subdomain (e.g., "v2-3-1" from "v2-3-1.turborepo.dev")
    const subdomain = host.replace(`.${PRODUCTION_DOMAIN}`, "");
    setVersion(subdomainToVersion(subdomain));
  }, []);

  if (!version) {
    return null;
  }

  return (
    <div className="mb-4 rounded-lg border border-blue-500/50 bg-blue-500/10 p-3 text-sm">
      <div className="flex items-center gap-2 font-medium text-blue-600 dark:text-blue-500">
        <InfoIcon className="size-4" />
        <span>Version: {version}</span>
      </div>
      <p className="mt-2 text-muted-foreground">
        <Link
          href={`https://${PRODUCTION_DOMAIN}`}
          className="font-medium text-blue-600 underline underline-offset-2 hover:text-blue-500 dark:text-blue-500 dark:hover:text-blue-400"
        >
          Visit the latest documentation.
        </Link>
      </p>
    </div>
  );
}
