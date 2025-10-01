"use client";

import Link from "next/link";
import { useSearchContext } from "fumadocs-ui/provider";
import { Menu } from "#components/icons/menu.tsx";
import { GithubLogo } from "#app/_components/logos.tsx";
import { gitHubRepoUrl } from "#lib/constants.ts";
import { Popover, PopoverContent, PopoverTrigger } from "../popover";
import type { Pages } from "../nav";
import { ThemeSwitcher } from "../nav/theme-switcher";
import { MagnifyingGlass } from "../icons/magnifying-glass";
import { XDotCom } from "../icons/x-dot-com";

export const MobileMenuTopLevel = ({ pages }: { pages: Pages }) => {
  const { setOpenSearch } = useSearchContext();

  return (
    <>
      <button
        className="block lg:hidden p-4 pr-2"
        onClick={() => {
          setOpenSearch(true);
        }}
      >
        <MagnifyingGlass />
      </button>
      <Popover>
        <PopoverTrigger className="p-4 pl-2">
          <Menu />
        </PopoverTrigger>
        <PopoverContent className="mr-4">
          {pages.map((page) => {
            return (
              <Link
                className="block p-1 text-sm hover:text-gray-800 dark:hover:text-gray-1000"
                href={page.href}
              >
                {page.tooltip}
              </Link>
            );
          })}

          <div className="flex flex-row mt-4 items-center justify-between">
            <div className="flex gap-4">
              <Link href={gitHubRepoUrl}>
                <GithubLogo className="w-5 h-5" />
              </Link>
              <Link href="https://x.com/turborepo">
                <XDotCom className="w-5 h-5" />
              </Link>
            </div>
            <ThemeSwitcher />
          </div>
        </PopoverContent>
      </Popover>
    </>
  );
};
