import type { JSX } from "react";
import { valid, gte } from "semver";
import { fetchDistTags } from "../app/api/binaries/version/route";

// This is an optimization to avoid fetching the latest version of turbo from npm
// It doesn't strictly need to always be up to date, but it will avoid a network
// request on page loads that use this component.
const STATIC_LATEST_RELEASE = "2.1.3";

if (!valid(STATIC_LATEST_RELEASE)) {
  throw new Error(`Invalid static version "${STATIC_LATEST_RELEASE}"`);
}

let cache: { latestVersion: string; timestamp: number } | null = null;
// 5 minutes
const CACHE_DURATION = 5 * 60 * 1000;

export async function InVersion({
  version,
  children,
}: {
  version: string;
  children: JSX.Element;
}): Promise<JSX.Element | null> {
  if (!valid(version)) {
    throw new Error(
      `Invalid version "${version}" provided to <InVersion /> component`
    );
  }

  if (gte(STATIC_LATEST_RELEASE, version)) {
    return children;
  }

  const now = Date.now();
  if (cache && now - cache.timestamp < CACHE_DURATION) {
    // eslint-disable-next-line no-console -- Purposeful.
    console.log("Using cached latest");
  } else {
    // Fetch latest version of turbo
    try {
      const tags = await fetchDistTags({ name: "turbo" });
      if (tags.latest) {
        cache = { latestVersion: tags.latest, timestamp: now };
      }
    } catch (err) {
      // eslint-disable-next-line no-console -- Purposeful.
      console.error("unable to fetch latest version", err);
      return null;
    }
  }

  if (!cache?.latestVersion) {
    return null;
  }

  if (gte(cache.latestVersion, version)) {
    return children;
  }

  return null;
}
