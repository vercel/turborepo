"use client";

import Link from "next/link";
import { useTreeContext } from "fumadocs-ui/provider";
import type { PageTree } from "fumadocs-core/server";
import { useEffect } from "react";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "#components/ui/collapsible.tsx";
import { ChevronRight } from "../icons/chevron-right";
import { cn } from "../cn";
import { itemVariants } from "./sidebar";
import { useLockBodyScroll } from "./use-lock-body-scroll";
import { useMobileMenuContext } from "./use-mobile-menu-context";
import { useIsMobile } from "./use-mobile";

export const MobileMenu = () => {
  const { root } = useTreeContext();
  const { openMobileMenu, setOpenMobileMenu } = useMobileMenuContext();
  const isMobile = useIsMobile();

  useEffect(() => {
    if (!isMobile) {
      setOpenMobileMenu(false);
    }
  }, [isMobile, setOpenMobileMenu]);

  useLockBodyScroll(openMobileMenu);

  return (
    <Collapsible
      className="group/collapsible absolute top-0 isolate z-10 block w-full border-b bg-background-200 px-4 text-base md:hidden"
      open={openMobileMenu}
      onOpenChange={setOpenMobileMenu}
    >
      <CollapsibleTrigger className="flex h-[var(--mobile-menu-height)] w-full items-center gap-x-2 text-gray-1000">
        <ChevronRight className="transition-transform group-data-[state=open]/collapsible:rotate-90 w-[14px] h-[14px]" />
        Menu
      </CollapsibleTrigger>
      <CollapsibleContent className="h-full">
        <div className="flex h-full flex-col py-3 max-h-[calc(100vh-98px)] overflow-auto">
          {renderMobileList(root.children, 1)}
        </div>
      </CollapsibleContent>
    </Collapsible>
  );
};

const MobileMenuLink = ({ item }: { item: PageTree.Item }) => {
  const { setOpenMobileMenu } = useMobileMenuContext();
  return (
    <Link
      href={item.url}
      key={item.url}
      onClick={() => {
        setOpenMobileMenu(false);
      }}
      className={cn(
        itemVariants(),
        "text-base font-normal text-gray-900 no-underline first-of-type:mt-1 hover:text-gray-1000 [&:not(:first-of-type)]:mt-0"
      )}
    >
      {item.name}
    </Link>
  );
};

export const getItemClass = (href: string | undefined) => {
  return href ? href.split("/").filter(Boolean).length > 3 : false;
};

export function renderMobileList(items: Array<PageTree.Node>, level: number) {
  return items.map((item, i) => {
    const id = `${item.type}_${i}`;

    switch (item.type) {
      case "separator":
        return (
          <span className={cn(itemVariants(), "text-base")} key={id}>
            {item.name}
          </span>
        );
      case "folder":
        return (
          <Collapsible key={id} className="group/folder flex flex-col gap-y-1">
            <CollapsibleTrigger asChild>
              <button
                type="button"
                className={cn(
                  itemVariants(),
                  "group/trigger text-base",
                  getItemClass(item.index?.url) ? "text-gray-900" : ""
                )}
              >
                {item.name}
                <ChevronRight
                  data-icon
                  className="ml-auto transition-transform group-data-[state=open]/folder:rotate-90 w-3 h-3"
                />
              </button>
            </CollapsibleTrigger>
            <CollapsibleContent>
              <div className="flex flex-col pb-1">
                {renderMobileList(item.children, level + 1)}
              </div>
            </CollapsibleContent>
          </Collapsible>
        );
      default:
        return <MobileMenuLink key={id} item={item} />;
    }
  });
}
