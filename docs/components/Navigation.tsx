import { Navbar } from "nextra-theme-docs";
import { useTurboSite } from "./SiteSwitcher";

function Navigation(props) {
  const site = useTurboSite();

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

  const lastItem = props.items[props.items.length - 1];
  if (lastItem?.id !== "contextual-enterprise") {
    props.items.push({
      title: "Enterprise",
      newWindow: true,
      // https://github.com/shuding/nextra/issues/1028
      route: "enterprise",
      href: `https://vercel.com/${
        site === "repo" ? "solutions/turborepo" : "contact/sales"
      }?utm_source=turbo.build&utm_medium=referral&utm_campaign=header-enterpriseLink`,
      id: "contextual-enterprise",
      key: "contextual-enterprise",
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
