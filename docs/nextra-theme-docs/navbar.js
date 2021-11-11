import React from "react";
import cn from "classnames";
import Link from "next/link";
import { useRouter } from "next/router";

import renderComponent from "./utils/render-component";
import { getFSRoute } from "./utils/get-fs-route";
import useMenuContext from "./utils/menu-context";

import Search from "./search";
import StorkSearch from "./stork-search";
import GitHubIcon from "./github-icon";
import ThemeSwitch from "./theme-switch";
import LocaleSwitch from "./locale-switch";
import DiscordIcon from "./discord-icon";

export default function Navbar({
  config,
  isRTL,
  flatDirectories,
  flatPageDirectories,
}) {
  const { locale, asPath } = useRouter();
  const activeRoute = getFSRoute(asPath, locale).split("#")[0];
  const { menu, setMenu } = useMenuContext();

  return (
    <>
      <div className="px-6 py-2 text-white bg-black dark:bg-white dark:text-black">
        <a
          href="https://vercel.com/home?utm_source=next-site&amp;utm_medium=banner&amp;utm_campaign=next-website"
          target="_blank"
          rel="noopener noreferrer"
          className="text-white dark:text-black"
          title="Go to the Vercel website"
        >
          <svg
            height="16"
            viewBox="0 0 283 64"
            fill="none"
            xmlns="http://www.w3.org/2000/svg"
          >
            <path
              d="M141.04 16c-11.04 0-19 7.2-19 18s8.96 18 20 18c6.67 0 12.55-2.64 16.19-7.09l-7.65-4.42c-2.02 2.21-5.09 3.5-8.54 3.5-4.79 0-8.86-2.5-10.37-6.5h28.02c.22-1.12.35-2.28.35-3.5 0-10.79-7.96-17.99-19-17.99zm-9.46 14.5c1.25-3.99 4.67-6.5 9.45-6.5 4.79 0 8.21 2.51 9.45 6.5h-18.9zM248.72 16c-11.04 0-19 7.2-19 18s8.96 18 20 18c6.67 0 12.55-2.64 16.19-7.09l-7.65-4.42c-2.02 2.21-5.09 3.5-8.54 3.5-4.79 0-8.86-2.5-10.37-6.5h28.02c.22-1.12.35-2.28.35-3.5 0-10.79-7.96-17.99-19-17.99zm-9.45 14.5c1.25-3.99 4.67-6.5 9.45-6.5 4.79 0 8.21 2.51 9.45 6.5h-18.9zM200.24 34c0 6 3.92 10 10 10 4.12 0 7.21-1.87 8.8-4.92l7.68 4.43c-3.18 5.3-9.14 8.49-16.48 8.49-11.05 0-19-7.2-19-18s7.96-18 19-18c7.34 0 13.29 3.19 16.48 8.49l-7.68 4.43c-1.59-3.05-4.68-4.92-8.8-4.92-6.07 0-10 4-10 10zm82.48-29v46h-9V5h9zM36.95 0L73.9 64H0L36.95 0zm92.38 5l-27.71 48L73.91 5H84.3l17.32 30 17.32-30h10.39zm58.91 12v9.69c-1-.29-2.06-.49-3.2-.49-5.81 0-10 4-10 10V51h-9V17h9v9.2c0-5.08 5.91-9.2 13.2-9.2z"
              fill="currentColor"
            ></path>
          </svg>
        </a>
      </div>
      <nav className="flex items-center bg-white z-20 sticky top-0 left-0 right-0 h-16 border-b border-gray-200 px-6 dark:bg-dark dark:border-gray-900 bg-opacity-[.97] dark:bg-opacity-100">
        <div className="flex items-center w-full mr-2">
          <Link href="/">
            <a className="inline-flex items-center text-current no-underline hover:opacity-75">
              {renderComponent(config.logo, { locale })}
            </a>
          </Link>
        </div>

        {flatPageDirectories
          ? flatPageDirectories.map((page) => {
              if (page.hidden) return null;

              let href = page.route;

              // If it's a directory
              if (page.children) {
                href = page.firstChildRoute;
              }

              return (
                <Link href={href} key={page.route}>
                  <a
                    className={cn(
                      "no-underline whitespace-nowrap mr-4 hidden md:inline-block",
                      page.route === activeRoute ||
                        activeRoute.startsWith(page.route + "/")
                        ? "text-current"
                        : "text-gray-500"
                    )}
                  >
                    {page.title}
                  </a>
                </Link>
              );
            })
          : null}
        <a
          href="https://vercel.com/contact/sales?"
          className={cn(
            "no-underline whitespace-nowrap mr-4 hidden md:inline-block",
            "text-gray-500"
          )}
        >
          Enterprise
        </a>
        <div className="flex-1">
          <div className="hidden mr-2 md:inline-block">
            {config.customSearch ||
              (config.search ? (
                config.unstable_stork ? (
                  <StorkSearch />
                ) : (
                  <Search directories={flatDirectories} />
                )
              ) : null)}
          </div>
        </div>

        {config.darkMode ? <ThemeSwitch /> : null}

        {config.i18n ? (
          <LocaleSwitch options={config.i18n} isRTL={isRTL} />
        ) : null}

        {config.projectLink || config.github ? (
          <a
            className="p-2 text-current"
            href={config.projectLink || config.github}
            target="_blank"
            rel="noreferrer"
          >
            {config.projectLinkIcon ? (
              renderComponent(config.projectLinkIcon, { locale })
            ) : (
              <GitHubIcon height={24} />
            )}
          </a>
        ) : null}

        {config.projectChatLink ? (
          <a
            className="p-2 text-current"
            href={config.projectChatLink}
            target="_blank"
            rel="noreferrer"
          >
            {config.projectChatLinkIcon ? (
              renderComponent(config.projectChatLinkIcon, { locale })
            ) : (
              <DiscordIcon height={24} />
            )}
          </a>
        ) : null}

        <button className="block p-2 md:hidden" onClick={() => setMenu(!menu)}>
          <svg
            fill="none"
            width="24"
            height="24"
            viewBox="0 0 24 24"
            stroke="currentColor"
          >
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth={2}
              d="M4 6h16M4 12h16M4 18h16"
            />
          </svg>
        </button>

        <div className="-mr-2" />
      </nav>
    </>
  );
}
