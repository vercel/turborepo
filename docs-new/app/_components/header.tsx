"use client";

import Link from "next/link";
import { SiteSwitcher } from "./site-switcher";
import styles from "./header-logo.module.css";
import { TurboAnimated } from "./logos/TurboAnimated";
import { LogoContext } from "./logo-context";
import { useTurboSite } from "../_components/site-switcher";

export function Header(props) {
  const site = useTurboSite();

  /*
    Inject a dynamic docs link when NOT on root
    1. Points to /repo/docs when on /repo
    2. Points to /pack/docs when on /pack
  */

  // I need to rebuild the links.
  return (
    <header className="flex items-center px-4 py-2 sticky top-0 z-50 backdrop-blur bg-white/30 dark:bg-black/30">
      <div className="flex items-center">
        <HeaderLogo />
      </div>
      <p>links</p>
    </header>
  );
}

export function HeaderLogo() {
  return (
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
    </>
  );
}
