import { Popover, PopoverContent, PopoverTrigger } from "../popover";
import { Menu } from "#/components/icons/menu";
import { Pages } from "../nav";
import Link from "next/link";
import { GithubLogo } from "@/app/_components/logos";
import { gitHubRepoUrl } from "@/lib/constants";
import { ThemeSwitcher } from "../nav/theme-switcher";

export const MobileMenu = ({ pages }: { pages: Pages }) => {
  return (
    <Popover>
      <PopoverTrigger>
        <Menu />
      </PopoverTrigger>
      <PopoverContent className="mr-4">
        {pages.map((page) => {
          return (
            <Link className="block p-1 text-sm" href={page.href}>
              {page.tooltip}
            </Link>
          );
        })}

        <div className="flex flex-row mt-4 items-center justify-between">
          <Link href={gitHubRepoUrl}>
            <GithubLogo className="w-5 h-5" />
          </Link>

          <ThemeSwitcher />
        </div>
      </PopoverContent>
    </Popover>
  );
};
