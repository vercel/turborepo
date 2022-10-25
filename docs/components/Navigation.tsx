import { useRouter } from "next/router";
import { Navbar } from "nextra-theme-docs";
import { useTurboSite } from "./SiteSwitcher";

function Navigation(props) {
  const site = useTurboSite();
  const router = useRouter();

  /*
    Inject a dynamic docs link when NOT on root
    1. Points to /repo/docs when on /repo
    2. Points to /pack/docs when on /pack
  */
  const leadingItem = props.items[0];
  if (leadingItem?.id !== "contextual-docs" && site) {
    props.items.unshift({
      title: "Docs",
      type: "page",
      route: `/${site}/docs`,
      id: "contextual-docs",
      key: "contextual-docs",
    });
  }

  // remove the top level repo and pack links
  const headerItems = props.items.filter((item) => {
    return item.name !== "repo" && item.name !== "pack";
  });

  // items last to override the default
  return <Navbar {...props} items={headerItems} />;
}

export default Navigation;
