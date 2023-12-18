import { useState, useEffect } from "react";
import { useRouter } from "next/router";
import { useConfig, useTheme, type DocsThemeConfig } from "nextra-theme-docs";
import { Footer } from "./components/Footer";
import Navigation from "./components/Navigation";
import HeaderLogo from "./components/HeaderLogo";
import { ExtraContent } from "./components/ExtraContent";
import { Discord, Github } from "./components/Social";

const SITE_ROOT = "https://turbo.build";

const config: DocsThemeConfig = {
  sidebar: {
    defaultMenuCollapseLevel: 1,
    toggleButton: true,
  },
  docsRepositoryBase: "https://github.com/vercel/turbo/blob/main/docs",
  useNextSeoProps: function SEO() {
    const router = useRouter();
    const { frontMatter } = useConfig();

    let section = "Turbo";
    if (router.pathname.startsWith("/pack")) {
      section = "Turbopack";
    }
    if (router.pathname.startsWith("/repo")) {
      section = "Turborepo";
    }

    // only show section if we're not on a landing page (these show as "Index")
    let titleTemplate = `%s – ${section}`;
    if (router.pathname === "/repo") {
      titleTemplate = `Turborepo`;
    }
    if (router.pathname === "/pack") {
      titleTemplate = `Turbopack`;
    }
    if (router.pathname === "/") {
      titleTemplate = `Turbo`;
    }

    const defaultTitle = frontMatter.overrideTitle || section;

    return {
      description: frontMatter.description,
      defaultTitle,
      titleTemplate,
    };
  },
  gitTimestamp({ timestamp }) {
    // eslint-disable-next-line react-hooks/rules-of-hooks
    const [dateString, setDateString] = useState(timestamp.toISOString());

    // eslint-disable-next-line react-hooks/rules-of-hooks
    useEffect(() => {
      try {
        setDateString(
          timestamp.toLocaleDateString(navigator.language, {
            day: "numeric",
            month: "long",
            year: "numeric",
          })
        );
      } catch (e) {
        // Ignore errors here; they get the ISO string.
        // At least one person out there has manually misconfigured navigator.language.
      }
    }, [timestamp]);

    return <>Last updated on {dateString}</>;
  },
  toc: {
    float: true,
    backToTop: true,
    extraContent: ExtraContent,
  },
  // font: false,
  logo: HeaderLogo,
  logoLink: false,
  head: function Head() {
    const router = useRouter();
    const { systemTheme = "dark" } = useTheme();
    const { frontMatter } = useConfig();
    const fullUrl =
      router.asPath === "/" ? SITE_ROOT : `${SITE_ROOT}${router.asPath}`;

    const asPath = router.asPath;

    let ogUrl;

    if (asPath === "/") {
      ogUrl = `${SITE_ROOT}/api/og`;
    } else if (frontMatter?.ogImage) {
      ogUrl = `${SITE_ROOT}${frontMatter.ogImage}`;
    } else {
      const type = asPath.startsWith("/repo")
        ? "repo"
        : asPath.startsWith("/pack")
        ? "pack"
        : "";
      const title = frontMatter.title
        ? `&title=${encodeURIComponent(frontMatter.title)}`
        : "";

      ogUrl = `${SITE_ROOT}/api/og?type=${type}${title}`;
    }

    return (
      <>
        <meta content="width=device-width, initial-scale=1.0" name="viewport" />
        <link
          href={`/images/favicon-${systemTheme}/apple-touch-icon.png`}
          rel="apple-touch-icon"
          sizes="180x180"
        />
        <link
          href={`/images/favicon-${systemTheme}/favicon-32x32.png`}
          rel="icon"
          sizes="32x32"
          type="image/png"
        />
        <link
          href={`/images/favicon-${systemTheme}/favicon-16x16.png`}
          rel="icon"
          sizes="16x16"
          type="image/png"
        />
        <link
          color="#000000"
          href={`/images/favicon-${systemTheme}/safari-pinned-tab.svg`}
          rel="mask-icon"
        />
        <link
          href={`/images/favicon-${systemTheme}/favicon.ico`}
          rel="shortcut icon"
        />
        <meta content="#000000" name="msapplication-TileColor" />
        <meta content="#000" name="theme-color" />
        <meta content="summary_large_image" name="twitter:card" />
        <meta content="@turborepo" name="twitter:site" />
        <meta content="@turborepo" name="twitter:creator" />
        <meta content="website" property="og:type" />
        <meta content={fullUrl} property="og:url" />
        <link href={fullUrl} rel="canonical" />
        <meta content={ogUrl} property="twitter:image" />
        <meta content={ogUrl} property="og:image" />
        <meta content="en_IE" property="og:locale" />
        <meta content="Turbo" property="og:site_name" />
        <link as="document" href="/repo" rel="prefetch" />
        <link as="document" href="/repo/docs" rel="prefetch" />
        <link as="document" href="/pack" rel="prefetch" />
        <link as="document" href="/pack/docs" rel="prefetch" />
        <link
          href="https://turbo.build/feed.xml"
          rel="alternate"
          title="Turbo Blog"
          type="application/rss+xml"
        />
      </>
    );
  },
  i18n: [],
  editLink: {
    text: "Edit this page on GitHub",
  },
  navbar: {
    component: Navigation,
    extraContent: (
      <>
        <Github />
        <Discord />
      </>
    ),
  },
  search: {
    placeholder: "Search documentation…",
  },
  footer: {
    component: Footer,
  },
  nextThemes: {
    defaultTheme: "dark",
  },
};

export default config;
