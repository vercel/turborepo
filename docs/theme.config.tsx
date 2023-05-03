import { useState, useEffect } from "react";
import { useRouter } from "next/router";
import { useConfig, useTheme, type DocsThemeConfig } from "nextra-theme-docs";
import { Footer } from "./components/Footer";
import Navigation from "./components/Navigation";
import HeaderLogo from "./components/HeaderLogo";
import ExtraContent from "./components/ExtraContent";
import { Discord, Github } from "./components/Social";

const SITE_ROOT = "https://turbo.build";

const config: DocsThemeConfig = {
  sidebar: {
    defaultMenuCollapseLevel: 10000,
  },
  docsRepositoryBase: "https://github.com/vercel/turbo/blob/main/docs",
  useNextSeoProps: function SEO() {
    const router = useRouter();
    const { frontMatter } = useConfig();

    let section = "Turbo";
    if (router?.pathname.startsWith("/pack")) {
      section = "Turbopack";
    }
    if (router?.pathname.startsWith("/repo")) {
      section = "Turborepo";
    }

    const defaultTitle = frontMatter.overrideTitle || section;

    return {
      description: frontMatter.description,
      defaultTitle,
      titleTemplate: `%s – ${section}`,
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
        <meta name="viewport" content="width=device-width, initial-scale=1.0" />
        <link
          rel="apple-touch-icon"
          sizes="180x180"
          href={`/images/favicon-${systemTheme}/apple-touch-icon.png`}
        />
        <link
          rel="icon"
          type="image/png"
          sizes="32x32"
          href={`/images/favicon-${systemTheme}/favicon-32x32.png`}
        />
        <link
          rel="icon"
          type="image/png"
          sizes="16x16"
          href={`/images/favicon-${systemTheme}/favicon-16x16.png`}
        />
        <link
          rel="mask-icon"
          href={`/images/favicon-${systemTheme}/safari-pinned-tab.svg`}
          color="#000000"
        />
        <link
          rel="shortcut icon"
          href={`/images/favicon-${systemTheme}/favicon.ico`}
        />
        <meta name="msapplication-TileColor" content="#000000" />
        <meta name="theme-color" content="#000" />
        <meta name="twitter:card" content="summary_large_image" />
        <meta name="twitter:site" content="@turborepo" />
        <meta name="twitter:creator" content="@turborepo" />
        <meta property="og:type" content="website" />
        <meta property="og:url" content={fullUrl} />
        <link rel="canonical" href={fullUrl} />
        <meta property="twitter:image" content={ogUrl} />
        <meta property="og:image" content={ogUrl} />
        <meta property="og:locale" content="en_IE" />
        <meta property="og:site_name" content="Turbo" />
        <link rel="prefetch" href="/repo" as="document" />
        <link rel="prefetch" href="/repo/docs" as="document" />
        <link rel="prefetch" href="/pack" as="document" />
        <link rel="prefetch" href="/pack/docs" as="document" />
        <link
          rel="alternate"
          type="application/rss+xml"
          title="Turbo Blog"
          href="https://turbo.build/feed.xml"
        />
      </>
    );
  },
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
