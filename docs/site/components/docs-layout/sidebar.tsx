"use client";

import Link, { type LinkProps } from "next/link";
import { usePathname } from "next/navigation";
import {
  createContext,
  type HTMLAttributes,
  useContext,
  useLayoutEffect,
  useMemo,
  useState,
} from "react";
import { cva } from "class-variance-authority";
import type { ScrollAreaProps } from "@radix-ui/react-scroll-area";
import { useOnChange } from "fumadocs-core/utils/use-on-change";
import {
  SidebarGroupLabel,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarMenuSub,
  SidebarProvider,
} from "#components/ui/sidebar.tsx";
import {
  Collapsible,
  CollapsibleTrigger,
  CollapsibleContent,
} from "#components/ui/collapsible.tsx";
import { ScrollArea, ScrollViewport } from "../ui/scroll-area";
import { cn } from "../cn";
import { ChevronRight } from "../icons/chevron-right";
import { FolderProvider, useFolderContext } from "./use-folder-context";

export function isActive(
  url: string,
  pathname: string,
  nested = true
): boolean {
  return url === pathname || (nested && pathname.startsWith(`${url}/`));
}

export const itemVariants = cva(
  "flex h-auto w-full items-center p-0 text-sm py-2 font-medium text-gray-1000 data-[active=true]:text-blue-700 dark:data-[active=true]:text-blue-600 [&:not(:first-of-type)]:mt-0 [overflow-wrap:anywhere] transition-colors duration-100"
);

export const getItemClass = (href: string | undefined) => {
  const hasMoreThanThreeSegments = href
    ? href.split("/").filter(Boolean).length > 3
    : false;

  return (className?: string) =>
    cn(
      itemVariants(),
      hasMoreThanThreeSegments && "py-1 text-gray-900",
      className
    );
};

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
  const active = isActive(String(href), pathname, false);
  const itemClasses = getItemClass(String(href));

  useLayoutEffect(() => {
    if (active) {
      setOpenFolder(true);
    }
  }, [active, setOpenFolder]);

  return (
    <div className="flex items-center w-full">
      <Link
        href={href}
        data-active={active}
        className={itemClasses(className)}
        prefetch={prefetch}
        {...props}
      >
        {children}
      </Link>
      <button
        onClick={() => {
          setOpenFolder(!openFolder);
        }}
        className="ml-auto p-1 hover:bg-gray-100 dark:hover:bg-gray-800 rounded-md transition-colors"
        aria-label={openFolder ? "Collapse section" : "Expand section"}
      >
        <ChevronRight
          data-icon
          className={cn(
            "transition-transform h-3 w-3",
            openFolder ? "rotate-90" : ""
          )}
        />
      </button>
    </div>
  );
};

export const SidebarFolderContent = ({
  children,
}: {
  children: React.ReactNode;
}) => {
  return (
    <CollapsibleContent>
      <SidebarMenuSub className="m-0 my-2.5 flex flex-col border-none p-0">
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
  const active = isActive(String(href), pathname, false);
  const { prefetch } = useInternalContext();

  return (
    <SidebarMenuItem>
      <SidebarMenuButton
        asChild
        className="m-0 flex h-auto w-full rounded-md p-0 text-sm font-normal hover:text-gray-1000 data-[active=true]:font-normal data-[active=true]:text-blue-700 dark:data-[active=true]:text-blue-600"
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
  const itemClasses = getItemClass(undefined);
  return (
    <SidebarGroupLabel {...props} className={itemClasses(className)}>
      {children}
    </SidebarGroupLabel>
  );
}
