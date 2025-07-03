"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import { useSearchContext } from "fumadocs-ui/provider";
import { VercelLogo } from "#app/_components/logos.tsx";
import { LogoGitHub } from "#components/icons/logo-github.tsx";
import {
  TurborepoWordmarkDark,
  TurborepoWordmarkLight,
} from "#components/icons/turborepo-wordmark.tsx";
import { MagnifyingGlass } from "#components/icons/magnifying-glass.tsx";
import { cn } from "../cn";
import { ForwardSlash } from "../icons/ForwardSlash";
import { Button } from "../button";
import { FeedbackWidget } from "../feedback-widget";
import { MobileMenuTopLevel } from "../docs-layout/mobile-menu-top-level";
import {
  NavigationMenu,
  NavigationMenuIndicator,
  NavigationMenuItem,
  NavigationMenuLink,
  NavigationMenuList,
} from "./navigation-menu";

export const PAGES = [
  {
    href: "/docs",
    tooltip: "Docs",
    name: "docs",
  },
  {
    href: "/blog",
    tooltip: "Blog",
    name: "blog",
  },
  {
    href: "/showcase",
    tooltip: "Showcase",
    name: "showcase",
  },
  {
    href: "https://vercel.com/contact/sales?utm_source=turborepo.com&utm_medium=referral&utm_campaign=header-enterpriseLink",
    tooltip: "Enterprise",
    name: "enterprise",
  },
] as const;
export type Pages = typeof PAGES;

function HomeLinks() {
  return (
    <div className="flex items-center gap-2">
      <Link href="https://vercel.com/" rel="noopener" target="_blank">
        <VercelLogo className="-translate-y-[0.5px] w-[18px] h-[18px]" />
      </Link>

      <ForwardSlash />

      <Link className="flex flex-row items-center gap-2" href="/">
        <TurborepoWordmarkDark className="h-[24px] w-auto hidden dark:block" />
        <TurborepoWordmarkLight className="h-[24px] w-auto dark:hidden" />
      </Link>
    </div>
  );
}

export const Navigation = () => {
  const pathname = usePathname();
  const pageFromRoute = pathname ? pathname.split("/")[1] : "";

  const { hotKey, setOpenSearch } = useSearchContext();

  return (
    <>
      <div className="sticky top-0 z-40 flex h-[var(--nav-height)] justify-between border-b bg-background-100 px-4 pr-0 md:pr-4">
        <div className="flex w-full select-none flex-row items-center">
          <div className="flex flex-shrink-0 flex-row items-center gap-2">
            <HomeLinks />
          </div>

          <div className="ml-auto flex md:hidden">
            <MobileMenuTopLevel pages={PAGES} />
          </div>
          <div className="hidden md:flex w-full justify-end md:justify-start md:pl-6">
            <NavigationMenu>
              <NavigationMenuList className="h-14 gap-3">
                {PAGES.map((page) => (
                  <NavigationMenuItem key={page.href} className="h-full">
                    <NavigationMenuLink
                      asChild
                      className="flex h-full items-center"
                    >
                      <Link
                        href={page.href}
                        className={cn(
                          "text-sm text-gray-900 transition-colors duration-100 hover:text-gray-1000 data-[active=true]:text-gray-1000"
                        )}
                        data-active={pageFromRoute === page.name}
                        scroll={page.href !== "/docs"}
                      >
                        {page.tooltip}
                      </Link>
                    </NavigationMenuLink>
                  </NavigationMenuItem>
                ))}
                <NavigationMenuIndicator />
              </NavigationMenuList>
            </NavigationMenu>
          </div>
        </div>

        <button
          className="hidden p-4 pr-2 md:pr-4 md:block lg:hidden"
          onClick={() => {
            setOpenSearch(true);
          }}
        >
          <MagnifyingGlass />
        </button>

        <div className="hidden items-center gap-2 md:flex">
          <Button
            aria-label="Search…"
            variant="secondary"
            size="sm"
            className="group border flex-row !font-normal !text-gray-800 hover:!text-gray-1000 hidden lg:block"
            onClick={() => {
              setOpenSearch(true);
            }}
          >
            <div className="text-start justify-between flex gap-2 lg:w-20 xl:w-24">
              <span>Search</span>
              <span className="inline-flex items-center justify-center rounded border border-gray-200 font-sans text-sm group-hover:border-gray-alpha-400">
                <kbd className="flex h-5 min-h-5 w-fit items-center px-1 py-0 text-center font-sans text-xs">
                  {hotKey.map((k, i) => (
                    <span key={`${i}-${k.key.toString()}`}>{k.display}</span>
                  ))}
                </kbd>
              </span>
            </div>
          </Button>

          <FeedbackWidget />
          <Button
            asChild
            size="sm"
            // @ts-expect-error - Button with asChild expects its children to have href but TypeScript doesn't recognize this pattern
            href="https://github.com/vercel/turborepo"
            className=""
          >
            <a>
              <LogoGitHub className="inline" />
              <span>GitHub</span>
            </a>
          </Button>
        </div>
      </div>
    </>
  );
};
