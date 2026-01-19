const MD_TRACKING_URL = process.env.MD_TRACKING_URL;
const MD_TRACKING_API_KEY = process.env.MD_TRACKING_API_KEY;

interface TrackMdRequestParams {
  path: string;
  userAgent: string | null;
  referer: string | null;
  acceptHeader: string | null;
}

/**
 * Track a markdown page request to the feedback-app analytics.
 * Fire-and-forget: errors are logged but don't affect the response.
 */
export async function trackMdRequest({
  path,
  userAgent,
  referer,
  acceptHeader
}: TrackMdRequestParams): Promise<void> {
  if (!MD_TRACKING_URL || !MD_TRACKING_API_KEY) {
    // Tracking not configured, skip silently
    return;
  }

  try {
    const response = await fetch(MD_TRACKING_URL, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        Authorization: `Bearer ${MD_TRACKING_API_KEY}`
      },
      body: JSON.stringify({
        path,
        source: "turborepo",
        userAgent,
        referer,
        acceptHeader
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
    // Fire-and-forget: don't let tracking errors affect the response
    console.error("MD tracking error:", error);
  }
}
