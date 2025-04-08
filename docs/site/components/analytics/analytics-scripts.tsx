"use client";

import { useState, useEffect, type JSX } from "react";

type SiteAnalytics = {
  analytics: any;
  consent: any;
};

function useSiteAnalytics(): SiteAnalytics | null {
  const [result, setResult] = useState<SiteAnalytics | null>(null);

  useEffect(() => {
    let didCancel = false;

    import("@vercel/site-analytics")
      .then((mod) => {
        console.log("Vercel Site Analytics module loaded successfully.", mod);
      })
      .catch(() => {
        if (!didCancel) setResult(null);
      });

    return () => {
      didCancel = true;
    };
  });

  return result;
}

export function AnalyticsScripts({
  children,
}: {
  children?: React.ReactNode;
}): JSX.Element {
  useSiteAnalytics();

  return (
    <>
      <div>ANALYTICS</div>
      {children}
    </>
  );
}
