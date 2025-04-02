import * as siteAnalyticsModule from "../../lib/site-analytics";

console.log("TESTING siteAnalyticsModule", siteAnalyticsModule);

export function AnalyticsScripts({
  children,
}: {
  children?: React.ReactNode;
}): JSX.Element {
  return (
    <>
      <div>ANALYTICS</div>
      {children}
    </>
  );
}
