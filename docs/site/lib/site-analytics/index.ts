const vercelSiteAnalyticsModule = await import("@vercel/site-analytics").catch(
  () => null
);

function getAnalyticsService() {
  if (!vercelSiteAnalyticsModule) {
    return [];
  }

  console.log(
    "Vercel Site Analytics module loaded successfully.",
    vercelSiteAnalyticsModule
  );
}

export const analytics = getAnalyticsService();
