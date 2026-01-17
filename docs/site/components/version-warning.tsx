"use client";

import { TriangleAlert } from "lucide-react";
import Link from "next/link";
import { useEffect, useState } from "react";

const PRODUCTION_DOMAIN = "turborepo.dev";
const NPM_REGISTRY_URL = "https://registry.npmjs.org/turbo/latest";

/**
 * Convert subdomain format to semver for comparison.
 * Subdomain format: "v2-3-1" -> "2.3.1"
 */
function subdomainToSemver(subdomain: string): string {
  return subdomain.replace(/^v/, "").replace(/-/g, ".");
}

/**
 * Compare two semver strings.
 * Returns true if `a` is older than `b`.
 */
function isOlderVersion(a: string, b: string): boolean {
  const aParts = a.split(".").map(Number);
  const bParts = b.split(".").map(Number);

  for (let i = 0; i < Math.max(aParts.length, bParts.length); i++) {
    const aVal = aParts[i] || 0;
    const bVal = bParts[i] || 0;
    if (aVal < bVal) return true;
    if (aVal > bVal) return false;
  }
  return false;
}

export function VersionWarning() {
  const [isOldVersion, setIsOldVersion] = useState(false);
  const [subdomainVersion, setSubdomainVersion] = useState("");

  useEffect(() => {
    const host = window.location.host;

    // Check if we're on a subdomain of turborepo.dev (e.g., v2-3-1.turborepo.dev)
    if (host === PRODUCTION_DOMAIN || !host.endsWith(`.${PRODUCTION_DOMAIN}`)) {
      return;
    }

    // Extract version from subdomain (e.g., "v2-3-1" from "v2-3-1.turborepo.dev")
    const subdomain = host.replace(`.${PRODUCTION_DOMAIN}`, "");
    setSubdomainVersion(subdomain);

    const currentSemver = subdomainToSemver(subdomain);

    // Fetch latest version from npm to compare
    fetch(NPM_REGISTRY_URL)
      .then((res) => res.json())
      .then((data) => {
        const latestVersion = data.version as string;

        if (isOlderVersion(currentSemver, latestVersion)) {
          setIsOldVersion(true);
        }
      })
      .catch(() => {
        // If we can't fetch npm, assume it's old to be safe
        setIsOldVersion(true);
      });
  }, []);

  if (!isOldVersion) {
    return null;
  }

  return (
    <div className="mb-4 rounded-lg border border-amber-500/50 bg-amber-500/10 p-3 text-sm">
      <div className="flex items-center gap-2 font-medium text-amber-600 dark:text-amber-500">
        <TriangleAlert className="size-4" />
        <span>Old Version ({subdomainVersion})</span>
      </div>
      <p className="mt-2 text-muted-foreground">
        You&apos;re viewing docs for an out-of-date version of Turborepo.{" "}
        <Link
          href={`https://${PRODUCTION_DOMAIN}`}
          className="block mt-2 font-medium text-amber-600 underline underline-offset-2 hover:text-amber-500 dark:text-amber-500 dark:hover:text-amber-400"
        >
          View latest docs →
        </Link>
      </p>
    </div>
  );
}
