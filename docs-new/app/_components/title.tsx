import { LogoContext } from "@/app/_components/logo-context";
import { TurboAnimated } from "@/app/_components/logos/TurboAnimated";
import { SiteSwitcher } from "@/app/_components/site-switcher";
import Link from "next/link";
import styles from "./header-logo.module.css";
import { NavbarLinks } from "./navbar-links";

export const NavbarChildren = () => (
  <>
    <LogoContext />
    <svg
      className="dark:text-[#333] text-[#eaeaea] ml-2 mr-1"
      data-testid="geist-icon"
      fill="none"
      height={24}
      shapeRendering="geometricPrecision"
      stroke="currentColor"
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth="1.5"
      viewBox="0 0 24 24"
    >
      <path d="M16.88 3.549L7.12 20.451" />
    </svg>

    <Link className="hover:opacity-75" href="/" title="Home">
      <TurboAnimated height={32} />
    </Link>
    <div className={styles.siteSwitcher}>
      <SiteSwitcher />
    </div>
    <div className="flex w-full justify-end">
      <NavbarLinks />
    </div>
  </>
);
