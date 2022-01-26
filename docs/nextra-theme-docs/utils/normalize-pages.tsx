import React from "react";
import { PageMapItem } from "nextra";
import getTitle from "title";

function getMetaTitle(meta: string | Record<string, any>) {
  if (typeof meta === "string") return meta;
  if (typeof meta === "object") return meta.title;
  return "";
}

function getMetaItemType(meta: string | Record<string, any>) {
  if (typeof meta === "object") return meta.type;
  return "docs";
}

function getMetaHidden(meta: string | Record<string, any>) {
  if (typeof meta === "object") return meta.hidden || false;
  return false;
}

export interface Item extends Omit<PageMapItem, "children"> {
  title: string;
  type: string;
  children?: Item[];
}
export interface PageItem extends Omit<PageMapItem, "children"> {
  title: string;
  type: string;
  children?: PageItem[];
  firstChildRoute?: string;
  hidden?: boolean;
}
export interface DocsItem extends Omit<PageMapItem, "children"> {
  title: string;
  type: string;
  children?: DocsItem[];
  firstChildRoute?: string;
}

export default function normalizePages({
  list,
  locale,
  defaultLocale,
  route,
  docsRoot = "",
}: {
  list: PageMapItem[];
  locale?: string;
  defaultLocale?: string;
  route: string;
  docsRoot?: string;
}) {
  let meta: string | Record<string, any> | undefined = "";
  for (let item of list) {
    if (item.name === "meta.json") {
      if (locale === item.locale) {
        meta = item.meta;
        break;
      }
      // fallback
      if (!meta) {
        meta = item.meta;
      }
    }
  }
  if (!meta) {
    meta = {};
  }

  const metaKeys = Object.keys(meta);
  const hasLocale = new Map();
  if (locale) {
    list.forEach((a) =>
      a.locale === locale ? hasLocale.set(a.name, true) : null
    );
  }

  // All directories
  const directories: Item[] = [];
  const flatDirectories: Item[] = [];

  // Docs directories
  const docsDirectories: DocsItem[] = [];
  const flatDocsDirectories: DocsItem[] = [];

  // Page directories
  const pageDirectories: PageItem[] = [];
  const flatPageDirectories: PageItem[] = [];

  let activeType: string | undefined = undefined;
  let activeIndex: number = 0;

  list
    .filter(
      (a) =>
        // not meta
        a.name !== "meta.json" &&
        // not hidden routes
        !a.name.startsWith("_") &&
        // locale matches, or fallback to default locale
        (a.locale === locale ||
          ((a.locale === defaultLocale || !a.locale) && !hasLocale.get(a.name)))
    )
    .sort((a, b) => {
      const indexA = metaKeys.indexOf(a.name);
      const indexB = metaKeys.indexOf(b.name);
      if (indexA === -1 && indexB === -1) return a.name < b.name ? -1 : 1;
      if (indexA === -1) return 1;
      if (indexB === -1) return -1;
      return indexA - indexB;
    })
    .forEach((a) => {
      if (typeof meta !== "object") return;
      const title = getMetaTitle(meta[a.name]) || getTitle(a.name);
      const type = getMetaItemType(meta[a.name]) || "docs";
      const hidden = getMetaHidden(meta[a.name]);

      // If the doc is under the active page root.
      const isCurrentDocsTree = type === "docs" && route.startsWith(docsRoot);

      if (a.route === route) {
        activeType = type;
        switch (type) {
          case "nav":
            activeIndex = flatPageDirectories.length;
            break;
          case "docs":
          default:
            if (isCurrentDocsTree) {
              activeIndex = flatDocsDirectories.length;
            }
        }
      }

      const normalizedChildren = a.children
        ? normalizePages({
            list: a.children,
            locale,
            defaultLocale,
            route,
            docsRoot: type === "nav" ? a.route : docsRoot,
          })
        : undefined;

      if (normalizedChildren) {
        if (
          normalizedChildren.activeIndex !== undefined &&
          normalizedChildren.activeType !== undefined
        ) {
          activeType = normalizedChildren.activeType;
          switch (activeType) {
            case "nav":
              activeIndex =
                flatPageDirectories.length + normalizedChildren.activeIndex;
              break;
            case "docs":
              activeIndex =
                flatDocsDirectories.length + normalizedChildren.activeIndex;
              break;
          }
        }
      }

      const item: Item = {
        ...a,
        title,
        type,
        children: normalizedChildren ? [] : undefined,
      };
      const docsItem: DocsItem = {
        ...a,
        title,
        type,
        children: normalizedChildren ? [] : undefined,
      };
      const pageItem: PageItem = {
        ...a,
        title,
        type,
        hidden,
        children: normalizedChildren ? [] : undefined,
      };

      if (normalizedChildren) {
        switch (type) {
          case "nav":
            pageItem.children.push(...normalizedChildren.pageDirectories);
            docsDirectories.push(...normalizedChildren.docsDirectories);

            // If it's a page with non-page children, we inject itself as a page too.
            if (
              !normalizedChildren.flatPageDirectories.length &&
              normalizedChildren.flatDirectories.length
            ) {
              pageItem.firstChildRoute =
                normalizedChildren.flatDirectories[0].route;
              flatPageDirectories.push(pageItem);
            }

            break;
          case "docs":
          default:
            if (isCurrentDocsTree) {
              Array.isArray(docsItem.children) &&
                docsItem.children.push(...normalizedChildren.docsDirectories);
              pageDirectories.push(...normalizedChildren.pageDirectories);
            }
        }

        flatDirectories.push(...normalizedChildren.flatDirectories);
        flatPageDirectories.push(...normalizedChildren.flatPageDirectories);

        flatDocsDirectories.push(...normalizedChildren.flatDocsDirectories);

        Array.isArray(item.children) &&
          item.children.push(...normalizedChildren.directories);
      } else {
        flatDirectories.push(item);
        switch (type) {
          case "nav":
            flatPageDirectories.push(pageItem);
            break;
          case "docs":
          default:
            if (isCurrentDocsTree) {
              flatDocsDirectories.push(docsItem);
            }
        }
      }

      directories.push(item);
      switch (type) {
        case "nav":
          pageDirectories.push(pageItem);
          break;
        case "docs":
        default:
          if (isCurrentDocsTree) {
            docsDirectories.push(docsItem);
          }
      }
    });

  return {
    activeType,
    activeIndex,
    directories,
    flatDirectories,
    docsDirectories,
    flatDocsDirectories,
    pageDirectories,
    flatPageDirectories,
  };
}
