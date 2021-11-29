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
      {config.banner
        ? renderComponent(config.projectLinkIcon, { locale })
        : null}
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
                      "no-underline whitespace-nowrap mr-6 hidden md:inline-block font-medium",
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
            "no-underline whitespace-nowrap mr-6 hidden md:inline-block font-medium",
            "text-gray-500"
          )}
        >
          Enterprise
        </a>
        <div className="flex-1">
          <div className="hidden mr-2 md:mr-4 md:inline-block">
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
