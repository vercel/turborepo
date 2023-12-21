import Link from "next/link";
import { SiteSwitcher } from "./SiteSwitcher";
import styles from "./header-logo.module.css";
import { TurboAnimated } from "./logos/TurboAnimated";
import { LogoContext } from "./LogoContext";

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
