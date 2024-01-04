import { RemoteCacheCounter } from "./RemoteCacheCounter";
import { useTurboSite } from "./SiteSwitcher";

export function ExtraContent() {
  const site = useTurboSite();

  if (site === "repo") {
    return <RemoteCacheCounter />;
  }

  return null;
}
