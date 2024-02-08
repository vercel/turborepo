import type { ReactElement } from "react";
import { useState, useEffect } from "react";
import { useRouter } from "next/router";
import { useConfig, useTheme, type DocsThemeConfig } from "nextra-theme-docs";
import dynamic from "next/dynamic";
import { Footer } from "./components/Footer";
import { Navigation } from "./components/Navigation";
import { HeaderLogo } from "./components/HeaderLogo";
import { ExtraContent } from "./components/ExtraContent";
import { Discord, Github } from "./components/Social";
import { Main } from "./components/Main";
import { Search } from "./components/Search";
import { pathHasToolbar } from "./lib/comments";

const NoSSRCommentsButton = dynamic(
  () => import("./components/CommentsButton").then((mod) => mod.CommentsButton),
  {
    ssr: false,
  }
);
const SITE_ROOT = "https://turbo.build";

interface Frontmatter {
  title: string;
  overrideTitle: string;
  description: string;
  ogImage: string;
}

const config: DocsThemeConfig = {
  sidebar: {
    defaultMenuCollapseLevel: 1,
    toggleButton: true,
  },
  docsRepositoryBase: "https://github.com/vercel/turbo/blob/main/docs",
  useNextSeoProps: function SEO() {
    const router = useRouter();
    const nextraConfig = useConfig();

    const frontMatter = nextraConfig.frontMatter as Frontmatter;

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
    // eslint-disable-next-line react-hooks/rules-of-hooks -- Following Nextra docs: https://nextra.site/docs/docs-theme/theme-configuration#last-updated-date
    const [dateString, setDateString] = useState(timestamp.toISOString());

    // eslint-disable-next-line react-hooks/rules-of-hooks -- Following Nextra docs: https://nextra.site/docs/docs-theme/theme-configuration#last-updated-date
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
    const nextraConfig = useConfig();

    const frontMatter = nextraConfig.frontMatter as Frontmatter;
    const fullUrl =
      router.asPath === "/" ? SITE_ROOT : `${SITE_ROOT}${router.asPath}`;

    const asPath = router.asPath;

    let ogUrl: string;

    if (asPath === "/") {
      ogUrl = `${SITE_ROOT}/api/og`;
    } else if (frontMatter.ogImage) {
      ogUrl = `${SITE_ROOT}${frontMatter.ogImage}`;
    } else {
      const type = () => {
        if (asPath.startsWith("/repo")) {
          return "repo";
        }

        if (asPath.startsWith("/pack")) {
          return "pack";
        }
        return "";
      };
      const title = frontMatter.title
        ? `&title=${encodeURIComponent(frontMatter.title)}`
        : "";

      ogUrl = `${SITE_ROOT}/api/og?type=${type()}${title}`;
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
    extraContent: (): JSX.Element => {
      // eslint-disable-next-line react-hooks/rules-of-hooks -- Nextra does not infer the type of extraContent correctly.
      const router = useRouter();

      return (
        <>
          {pathHasToolbar(router) ? (
            <div className="w-6 h-6 ml-2 rounded-tl-none rounded-full border-2 border-white">
              <NoSSRCommentsButton />
            </div>
          ) : null}
          <Github />
          <Discord />
        </>
      );
    },
  },
  components: {
    pre: (props: ReactElement) => {
      function getTextContent(elem: ReactElement | string): string {
        if (elem instanceof Array) {
          return elem.map(getTextContent).join("");
        }
        if (!elem) {
          return "";
        }
        if (typeof elem === "string") {
          return elem;
        }

        // eslint-disable-next-line @typescript-eslint/no-unsafe-assignment, @typescript-eslint/no-unsafe-member-access -- `any` is technically correct here.
        const children = elem.props.children;
        if (children instanceof Array) {
          return children.map(getTextContent).join("");
        }

        return getTextContent(children as ReactElement | string);
      }

      // Taken from original Nextra docs theme.
      // Only functional change is adding `data-pagefind-weight`.
      return (
        <div className="relative mt-6" id="custom-code-block">
          <pre
            className="nx-bg-primary-700/5 nx-mb-4 nx-overflow-x-auto nx-rounded-xl nx-subpixel-antialiased dark:nx-bg-primary-300/10 nx-text-[.9em] contrast-more:nx-border contrast-more:nx-border-primary-900/20 contrast-more:nx-contrast-150 contrast-more:dark:nx-border-primary-100/40 nx-py-4"
            {...props}
            data-pagefind-weight=".5"
          />
          <div className="nx-opacity-0 nx-transition [div:hover>&amp;]:nx-opacity-100 focus-within:nx-opacity-100 nx-flex nx-gap-1 nx-absolute nx-m-[11px] nx-right-0 nx-top-0">
            <button
              className="nextra-button nx-transition-all active:nx-opacity-50 nx-bg-primary-700/5 nx-border nx-border-black/5 nx-text-gray-600 hover:nx-text-gray-900 nx-rounded-md nx-p-1.5 dark:nx-bg-primary-300/10 dark:nx-border-white/10 dark:nx-text-gray-400 dark:hover:nx-text-gray-50 md:nx-hidden"
              title="Toggle word wrap"
              type="button"
            >
              <svg
                className="nx-pointer-events-none nx-h-4 nx-w-4"
                height="24"
                viewBox="0 0 24 24"
                width="24"
              >
                <path
                  d="M4 19h6v-2H4v2zM20 5H4v2h16V5zm-3 6H4v2h13.25c1.1 0 2 .9 2 2s-.9 2-2 2H15v-2l-3 3l3 3v-2h2c2.21 0 4-1.79 4-4s-1.79-4-4-4z"
                  fill="currentColor"
                />
              </svg>
            </button>
            <button
              className="nextra-button nx-transition-all active:nx-opacity-50 nx-bg-primary-700/5 nx-border nx-border-black/5 nx-text-gray-600 hover:nx-text-gray-900 nx-rounded-md nx-p-1.5 dark:nx-bg-primary-300/10 dark:nx-border-white/10 dark:nx-text-gray-400 dark:hover:nx-text-gray-50"
              onClick={() => {
                void navigator.clipboard.writeText(
                  // @ts-expect-error -- `any` is technically correct here.
                  // eslint-disable-next-line @typescript-eslint/no-unsafe-argument, @typescript-eslint/no-unsafe-member-access -- `any` is technically correct here.
                  getTextContent(props.children.props.children)
                );
              }}
              tabIndex={0}
              title="Copy code"
              type="button"
            >
              <svg
                className="nextra-copy-icon nx-pointer-events-none nx-h-4 nx-w-4"
                fill="none"
                height="24"
                stroke="currentColor"
                viewBox="0 0 24 24"
                width="24"
                xmlns="http://www.w3.org/2000/svg"
              >
                <rect
                  height="13"
                  rx="2"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth="2"
                  width="13"
                  x="9"
                  y="9"
                />
                <path
                  d="M5 15H4C2.89543 15 2 14.1046 2 13V4C2 2.89543 2.89543 2 4 2H13C14.1046 2 15 2.89543 15 4V5"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth="2"
                />
              </svg>
            </button>
          </div>
        </div>
      );
    },
  },
  main: (props) => <Main>{props.children}</Main>,
  search: {
    component: Search,
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
