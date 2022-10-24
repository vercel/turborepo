import TurboLogo, { TurboLogoCondensed } from "./logos/Turbo";
import SiteSwitcher from "./SiteSwitcher";
import Link from "next/link";
import styles from "./header-logo.module.css";

function HeaderLogo() {
  return (
    <>
      <Link href="/" passHref>
        <a className="hover:opacity-75">
          <TurboLogo height={32} className={styles.desktopLogo} />
          <TurboLogoCondensed height={32} className={styles.mobileLogo} />
        </a>
      </Link>
      <div className={styles.siteSwitcher}>
        <SiteSwitcher />
      </div>
    </>
  );
}

export default HeaderLogo;
