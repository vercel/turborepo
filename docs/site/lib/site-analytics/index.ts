const vercelSiteAnalyticsModule = await import("@vercel/site-analytics").catch(
  () => null
);

function getAnalyticsService() {
  if (!vercelSiteAnalyticsModule) {
    return [];
  }
}

export const analytics = getAnalyticsService();
