"use client";

import Link from "next/link";
import {
  NavigationMenu,
  NavigationMenuIndicator,
  NavigationMenuItem,
  NavigationMenuLink,
  NavigationMenuList,
} from "./navigation-menu";
import { cn } from "../cn";
import { usePathname } from "next/navigation";
import { VercelLogo } from "@/app/_components/logos";
import { LogoGitHub } from "#/components/icons/logo-github";
import { ForwardSlash } from "../icons/ForwardSlash";
import { Button } from "../button";
import { FeedbackWidget } from "../feedback-widget";
import { useSearchContext } from "fumadocs-ui/provider";
import { ThemeAwareImage } from "../theme-aware-image";
import { MobileMenu } from "../docs-layout/mobile-menu-top-level";

export const PAGES = [
  {
    href: "/repo/docs",
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
    href: "https://vercel.com/contact/sales?utm_source=turbo.build&utm_medium=referral&utm_campaign=header-enterpriseLink",
    tooltip: "Enterprise",
    name: "enterprise",
  },
] as const;
export type Pages = typeof PAGES;

const size = 24;

function HomeLinks() {
  return (
    <div className="flex items-center gap-2">
      <Link href="https://vercel.com/" rel="noopener" target="_blank">
        <VercelLogo className="-translate-y-[0.5px] w-[18px] h-[18px]" />
      </Link>

      <ForwardSlash className="w-[16px] h-[16px]" />

      <Link className="flex flex-row items-center gap-2" href="/">
        <ThemeAwareImage
          light={{
            src: "/images/product-icons/repo-light-32x32.png",
            alt: "Turborepo logo",
            props: {
              src: "/images/product-icons/repo-light-32x32.png",
              alt: "Turborepo logo",
              width: size,
              height: size,
            },
          }}
          dark={{
            src: "/images/product-icons/repo-dark-32x32.png",
            alt: "Turborepo logo",
            props: {
              src: "/images/product-icons/repo-dark-32x32.png",
              alt: "Turborepo logo",
              width: size,
              height: size,
            },
          }}
        />
        <div className="text-lg font-bold ml-2">Turborepo</div>
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
      <div className="sticky top-0 z-40 flex h-[var(--nav-height)] justify-between border-b bg-background-200 px-4">
        <div className="flex w-full select-none flex-row items-center">
          <div className="flex flex-shrink-0 flex-row items-center gap-2">
            <HomeLinks />
          </div>
          <div className="ml-auto">
            <MobileMenu pages={PAGES} />
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
                          "text-sm transition-colors duration-100 hover:text-gray-1000 data-[active=true]:text-gray-1000"
                        )}
                        data-active={pageFromRoute === page.name}
                        scroll={page.href !== "/repo/docs"}
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

        <div className="hidden items-center gap-2 md:flex">
          <Button
            aria-label="Search…"
            variant="secondary"
            size="sm"
            className="group flex-row !font-normal !text-gray-800 hover:!text-gray-1000"
            onClick={() => {
              setOpenSearch(true);
            }}
          >
            <div className="text-start justify-between flex gap-2 lg:w-20 xl:w-24">
              <span>Search…</span>
              <span className="inline-flex items-center justify-center rounded border border-gray-200 font-sans text-sm group-hover:border-gray-alpha-400">
                <kbd className="flex h-5 min-h-5 w-fit items-center px-1 py-0 text-center font-sans text-xs">
                  {hotKey.map((k, i) => (
                    <span key={`${i}-${k.key}`}>{k.display}</span>
                  ))}
                </kbd>
              </span>
            </div>
          </Button>

          <FeedbackWidget />
          <Button
            variant="default"
            asChild
            size="sm"
            // @ts-expect-error
            href="https://github.com/vercel/turborepo"
            className="hidden xl:flex"
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
