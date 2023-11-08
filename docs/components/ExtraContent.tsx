import RemoteCacheCounter from "./RemoteCacheCounter";
import { useTurboSite } from "./SiteSwitcher";

export default function ExtraContent() {
  const site = useTurboSite();

  if (site === "repo") {
    return <RemoteCacheCounter />;
  }
}
