"use client";

import React from "react";
import Link from "next/link";
import { useParams, usePathname } from "next/navigation";
import type { PageTree, TOCItemType } from "fumadocs-core/server";
import { findNeighbour } from "fumadocs-core/server";
import { useTreeContext, useTreePath } from "fumadocs-ui/provider";
import * as Base from "fumadocs-core/toc";
import { useActiveAnchor } from "fumadocs-core/toc";
import { repoDocsPages } from "#app/source.ts";
import { SidebarMenu } from "#components/ui/sidebar.tsx";
import { RemoteCacheCounter } from "#components/remote-cache-counter/index.tsx";
import { AlignmentLeft } from "../icons/alignment-left";
import { ChevronLeft } from "../icons/chevron-left";
import { ChevronRight } from "../icons/chevron-right";
import {
  SidebarFolder,
  SidebarFolderLink,
  SidebarItem,
  SidebarFolderContent,
  SidebarFolderTrigger,
  SidebarSeparator,
} from "./sidebar";

export const LayoutBody = ({
  children,
  isOpenApiSpec,
}: {
  children: React.ReactNode;
  isOpenApiSpec?: boolean;
}) => {
  return (
    <div
      id="nd-docs-layout"
      data-openapi={isOpenApiSpec}
      className="mx-auto mb-16 grid w-full max-w-screen-xl grid-cols-1 gap-x-6 md:grid-cols-[var(--sidebar-width)_minmax(0,1fr)]"
    >
      {children}
    </div>
  );
};

function renderSidebarList(
  items: Array<PageTree.Node>
): Array<React.ReactNode> {
  return items.map((item, i) => {
    const id = `${item.type}_${i}`;

    switch (item.type) {
      case "separator":
        return <SidebarSeparator key={id}>{item.name}</SidebarSeparator>;
      case "folder":
        return (
          <PageTreeFolder key={id} item={item}>
            {item.index ? (
              <SidebarFolderLink href={item.index.url}>
                {item.name}
              </SidebarFolderLink>
            ) : (
              <SidebarFolderTrigger>{item.name}</SidebarFolderTrigger>
            )}
            <SidebarFolderContent>
              {renderSidebarList(item.children)}
            </SidebarFolderContent>
          </PageTreeFolder>
        );
      default:
        return (
          <SidebarItem key={item.url} href={item.url}>
            {item.name}
          </SidebarItem>
        );
    }
  });
}

export const SidebarItems = () => {
  const { root } = useTreeContext();
  return (
    <SidebarMenu className="flex flex-col">
      {renderSidebarList(root.children)}
    </SidebarMenu>
  );
};

const PageTreeFolder = ({
  item,
  children,
}: {
  item: PageTree.Folder;
  children: React.ReactNode;
}) => {
  const path = useTreePath();
  return (
    <SidebarFolder defaultOpen={item.defaultOpen || path.includes(item)}>
      {children}
    </SidebarFolder>
  );
};

export const Footer = () => {
  const { root } = useTreeContext();
  const pathname = usePathname();
  const neighbours = findNeighbour(root, pathname);
  return (
    <div className="grid w-full grid-cols-2 gap-4 md:mt-4 md:pb-6">
      {neighbours.previous ? (
        <Link
          className="group flex w-full flex-col items-start gap-2 text-sm no-underline opacity-100 transition-colors [&>*]:hover:text-gray-1000 [&_svg]:hover:fill-gray-1000"
          href={neighbours.previous.url}
        >
          <div className="flex items-center justify-center gap-x-1.5 text-gray-900 text-label-14">
            <ChevronLeft className="translate-y-px w-[10px] h-[10px]" />
            Previous
          </div>
          <span className="font-medium text-gray-1000 text-label-16">
            {neighbours.previous.name}
          </span>
        </Link>
      ) : null}
      {neighbours.next ? (
        <Link
          className="col-start-2 flex w-full flex-col items-end gap-2 text-sm no-underline opacity-100 transition-colors [&>*]:hover:text-gray-1000 [&_svg]:hover:fill-gray-1000"
          href={neighbours.next.url}
        >
          <div className="flex items-center justify-center gap-x-1.5 text-gray-900 text-label-14">
            Next <ChevronRight className="translate-y-px w-[10px] h-[10px]" />
          </div>
          <span className="font-medium text-gray-1000 text-label-16">
            {neighbours.next.name}
          </span>
        </Link>
      ) : null}
    </div>
  );
};

function getDepthClassName(depth: number) {
  switch (depth) {
    case 3:
      return "pl-3";
    case 4:
      return "pl-6";
    case 5:
      return "pl-9";
    default:
      return "";
  }
}

const TOCItem = ({ item }: { item: TOCItemType }) => {
  const activeAnchor = useActiveAnchor();
  const isActive = item.url.replace("#", "") === activeAnchor;

  return (
    <li className={`text-sm text-gray-900 ${getDepthClassName(item.depth)}`}>
      <Base.TOCItem
        data-active={isActive}
        href={item.url}
        className="data-[active=true]:text-blue-700 dark:data-[active=true]:text-blue-600"
      >
        {item.title}
      </Base.TOCItem>
    </li>
  );
};

export const TableOfContents = () => {
  const params = useParams<{ code: string; slug: Array<string> }>();
  const page = repoDocsPages.getPage(params.slug);
  if (!page) return null;
  const { data } = page;
  const ref = React.useRef<HTMLDivElement>(null);

  return (
    <>
      <Base.AnchorProvider toc={data.toc}>
        <Base.ScrollProvider containerRef={ref}>
          <span className="-ms-0.5 flex mb-2 items-center gap-x-1.5 text-sm font-medium text-gray-1000">
            <AlignmentLeft className="w-3 h-3" />
            On this page
          </span>
          <div className="max-h-[calc(100vh-300px)] overflow-auto">
            <ul className="flex flex-col gap-y-2.5 text-sm text-gray-900">
              {data.toc.map((item) => {
                return <TOCItem key={item.url} item={item} />;
              })}
            </ul>
          </div>
        </Base.ScrollProvider>
      </Base.AnchorProvider>
      <RemoteCacheCounter />
    </>
  );
};
