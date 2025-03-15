"use client";

import Link, { type LinkProps } from "next/link";
import { usePathname } from "next/navigation";
import {
  Collapsible,
  CollapsibleTrigger,
  CollapsibleContent,
} from "@/components/ui/collapsible";
import { ChevronRight } from "../icons/chevron-right";
import {
  SidebarGroupLabel,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarMenuSub,
  SidebarProvider,
} from "@/components/ui/sidebar";
import { FolderProvider, useFolderContext } from "./use-folder-context";
import {
  createContext,
  type HTMLAttributes,
  useContext,
  useLayoutEffect,
  useMemo,
  useState,
} from "react";
import { cva } from "class-variance-authority";
import { cn } from "../cn";
import { ScrollArea, ScrollViewport } from "../ui/scroll-area";
import type { ScrollAreaProps } from "@radix-ui/react-scroll-area";
import { useOnChange } from "fumadocs-core/utils/use-on-change";

export function isActive(
  url: string,
  pathname: string,
  nested = true
): boolean {
  return url === pathname || (nested && pathname.startsWith(`${url}/`));
}

export const itemVariants = cva(
  "flex h-auto w-full items-center p-0 text-sm font-medium text-gray-1000 data-[active=true]:text-blue-700 dark:data-[active=true]:text-blue-600 [&:not(:first-of-type)]:mt-2.5 [overflow-wrap:anywhere] transition-colors duration-100"
);
export interface SidebarProps extends HTMLAttributes<HTMLElement> {
  /**
   * Open folders by default if their level is lower or equal to a specific level
   * (Starting from 1)
   *
   * @defaultValue 0
   */
  defaultOpenLevel?: number;

  /**
   * Prefetch links
   *
   * @defaultValue true
   */
  prefetch?: boolean;
}

interface InternalContext {
  defaultOpenLevel: number;
  prefetch: boolean;
  // We don't really use levels, but future proofing
  level: number;
}

const Context = createContext<InternalContext | undefined>(undefined);
function useInternalContext(): InternalContext {
  const ctx = useContext(Context);
  if (!ctx) throw new Error("<Sidebar /> component required.");

  return ctx;
}

export const Sidebar = ({
  defaultOpenLevel = 0,
  prefetch = true,
  children,
}: SidebarProps) => {
  const context = useMemo<InternalContext>(() => {
    return {
      defaultOpenLevel,
      prefetch,
      level: 1,
    };
  }, [defaultOpenLevel, prefetch]);

  return (
    <Context.Provider value={context}>
      <SidebarProvider>{children}</SidebarProvider>
    </Context.Provider>
  );
};

export const SidebarFolder = ({
  defaultOpen,
  children,
}: {
  defaultOpen: boolean;
  children: React.ReactNode;
}) => {
  const [openFolder, setOpenFolder] = useState(defaultOpen);

  useOnChange(defaultOpen, (v) => {
    if (v) setOpenFolder(v);
  });

  return (
    <Collapsible
      className="group/collapsible"
      open={openFolder}
      onOpenChange={setOpenFolder}
    >
      <FolderProvider
        value={useMemo(() => ({ openFolder, setOpenFolder }), [openFolder])}
      >
        <ul className="list-none p-0">{children}</ul>
      </FolderProvider>
    </Collapsible>
  );
};

export function SidebarViewport(props: ScrollAreaProps) {
  return (
    <ScrollArea {...props} className={cn("h-full", props.className)}>
      <ScrollViewport
        style={{
          maskImage:
            "linear-gradient(to bottom, transparent, black 12px, black calc(100% - 12px), transparent 100%)",
          WebkitMaskImage:
            "linear-gradient(to bottom, transparent, black 12px, black calc(100% - 12px), transparent 100%)",
        }}
      >
        {props.children}
      </ScrollViewport>
    </ScrollArea>
  );
}

export const SidebarFolderTrigger = ({
  children,
}: {
  children: React.ReactNode;
}) => {
  const { openFolder } = useFolderContext();
  return (
    <SidebarMenuItem>
      <CollapsibleTrigger asChild>
        <SidebarMenuButton className="m-0 flex h-auto w-full items-center justify-between rounded-md p-0 text-sm font-medium text-gray-1000 hover:text-gray-1000">
          {children}
          <ChevronRight
            data-icon
            className={cn(
              "ml-auto transition-transform w-3 h-3",
              openFolder ? "rotate-90" : ""
            )}
          />
        </SidebarMenuButton>
      </CollapsibleTrigger>
    </SidebarMenuItem>
  );
};

export const SidebarFolderLink = ({
  href,
  className,
  children,
  ...props
}: LinkProps & {
  className?: string;
  children: React.ReactNode;
}) => {
  const { openFolder, setOpenFolder } = useFolderContext();
  const { prefetch } = useInternalContext();
  const pathname = usePathname();
  const active = href !== undefined && isActive(String(href), pathname, false);

  useLayoutEffect(() => {
    if (active) {
      setOpenFolder(true);
    }
  }, [active, setOpenFolder]);

  return (
    <Link
      href={href}
      data-active={active}
      className={cn(itemVariants(), className)}
      prefetch={prefetch}
      onClick={(e: any) => {
        if (
          // clicking on icon
          (e.target as HTMLElement).hasAttribute("data-icon") ||
          active
        ) {
          setOpenFolder(!openFolder);
          e.preventDefault();
        }
      }}
      {...props}
    >
      {children}
      <ChevronRight
        data-icon
        className={cn(
          "ml-auto transition-transform h-3 w-3",
          openFolder ? "rotate-90" : ""
        )}
      />
    </Link>
  );
};

export const SidebarFolderContent = ({
  children,
}: {
  children: React.ReactNode;
}) => {
  return (
    <CollapsibleContent>
      <SidebarMenuSub className="m-0 my-2.5 flex flex-col gap-y-2.5 border-none p-0">
        {children}
      </SidebarMenuSub>
    </CollapsibleContent>
  );
};

export const SidebarItem = ({
  href,
  children,
}: LinkProps & {
  children: React.ReactNode;
}) => {
  const pathname = usePathname();
  const active = href !== undefined && isActive(String(href), pathname, false);
  const { prefetch } = useInternalContext();

  return (
    <SidebarMenuItem>
      <SidebarMenuButton
        asChild
        className="m-0 flex h-auto w-full rounded-md p-0 text-sm font-normal text-gray-900 hover:text-gray-1000 data-[active=true]:font-normal data-[active=true]:text-blue-700 dark:data-[active=true]:text-blue-600"
      >
        <Link href={href} data-active={active} prefetch={prefetch}>
          <span className="truncate">{children}</span>
        </Link>
      </SidebarMenuButton>
    </SidebarMenuItem>
  );
};

export function SidebarSeparator({
  className,
  children,
  ...props
}: {
  className?: string;
  children: React.ReactNode;
}) {
  return (
    <SidebarGroupLabel {...props} className={cn(itemVariants(), className)}>
      {children}
    </SidebarGroupLabel>
  );
}
