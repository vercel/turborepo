import type { DetectionMethod } from "@vercel/agent-readability";
import { siteId } from "@/geistdocs";

const PLATFORM_URL = "https://geistdocs.com/md-tracking";

interface TrackMdRequestParams {
  acceptHeader: string | null;
  /** Detection method used to identify the agent (only for agent-rewrite requests) */
  detectionMethod?: DetectionMethod | null;
  path: string;
  referer: string | null;
  /** How the markdown was requested: 'md-url' for direct .md URLs, 'header-negotiated' for Accept header, 'agent-rewrite' for detected AI agents */
  requestType?: "md-url" | "header-negotiated" | "agent-rewrite";
  userAgent: string | null;
}

/**
 * Track a markdown page request via the geistdocs platform.
 * Fire-and-forget: errors are logged but don't affect the response.
 */
export async function trackMdRequest({
  path,
  userAgent,
  referer,
  acceptHeader,
  requestType,
  detectionMethod
}: TrackMdRequestParams): Promise<void> {
  try {
    const response = await fetch(PLATFORM_URL, {
      method: "POST",
      headers: {
        "Content-Type": "application/json"
      },
      body: JSON.stringify({
        path,
        siteId: siteId ?? "geistdocs-unknown",
        userAgent,
        referer,
        acceptHeader,
        requestType,
        detectionMethod
      })
    });

    if (!response.ok) {
      console.error(
        "MD tracking failed:",
        response.status,
        await response.text()
      );
    }
  } catch (error) {
    console.error("MD tracking error:", error);
  }
}
