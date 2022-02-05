import React, { useState, useEffect, useMemo, useCallback } from "react";
import cn from "classnames";
import Slugger from "github-slugger";
import { useRouter } from "next/router";
import Link from "next/link";
import innerText from "react-innertext";

import { useActiveAnchor } from "./misc/active-anchor";
import { getFSRoute } from "./utils/get-fs-route";
import useMenuContext from "./utils/menu-context";
import ArrowRight from "./icons/arrow-right";
import Search from "./flexsearch";
import { useConfig } from "./config";

const TreeState = new Map();
function Folder({ item, anchors }) {
  const { asPath, locale } = useRouter();
  const routeOriginal = getFSRoute(asPath, locale);
  const route = routeOriginal.split("#")[0];
  const active = route === item.route + "/" || route + "/" === item.route + "/";
  const { defaultMenuCollapsed } = useMenuContext();
  const open = TreeState[item.route] ?? !defaultMenuCollapsed;
  const [_, render] = useState(false);

  useEffect(() => {
    if (active) {
      TreeState[item.route] = true;
    }
  }, [active]);

  return (
    <li className={open ? "active" : ""}>
      <button
        onClick={() => {
          if (active) return;
          TreeState[item.route] = !open;
          render((x) => !x);
        }}
      >
        <span className="flex items-center justify-between gap-2">
          {item.title}
          <ArrowRight
            height="1em"
            className={cn(open ? "rotate-90" : "", "transition-transform")}
          />
        </span>
      </button>
      <div
        style={{
          display: open ? "initial" : "none",
        }}
      >
        {Array.isArray(item.children) && (
          <Menu
            directories={item.children}
            base={item.route}
            anchors={anchors}
          />
        )}
      </div>
    </li>
  );
}

function File({ item, anchors }) {
  const { setMenu } = useMenuContext();
  const { asPath, locale } = useRouter();
  const route = getFSRoute(asPath, locale);
  const active = route === item.route + "/" || route + "/" === item.route + "/";
  const slugger = new Slugger();
  const activeAnchor = useActiveAnchor();

  const title = item.title;
  // if (item.title.startsWith('> ')) {
  // title = title.substr(2)
  if (anchors && anchors.length) {
    if (active) {
      let activeIndex = 0;
      const anchorInfo = anchors.map((anchor, i) => {
        const text = innerText(anchor) || "";
        const slug = slugger.slug(text);
        if (activeAnchor[slug] && activeAnchor[slug].isActive) {
          activeIndex = i;
        }
        return { text, slug };
      });

      return (
        <li className={active ? "active" : ""}>
          <Link href={item.route}>
            <a>{title}</a>
          </Link>
          <ul>
            {anchors.map((_, i) => {
              const { slug, text } = anchorInfo[i];
              const isActive = i === activeIndex;

              return (
                <li key={`a-${slug}`}>
                  <a
                    href={"#" + slug}
                    onClick={() => setMenu(false)}
                    className={isActive ? "active-anchor" : ""}
                  >
                    <span className="flex text-sm">
                      <span className="opacity-25">#</span>
                      <span className="mr-2"></span>
                      <span className="inline-block">{text}</span>
                    </span>
                  </a>
                </li>
              );
            })}
          </ul>
        </li>
      );
    }
  }

  return (
    <li className={active ? "active" : ""}>
      <Link href={item.route}>
        <a onClick={() => setMenu(false)}>{title}</a>
      </Link>
    </li>
  );
}

function Menu({ directories, anchors }) {
  const config = useConfig();
  return (
    <ul>
      {directories.map((item) => {
        if (item.name === "blog") {
          return <File key={item.name} item={item} anchors={anchors} />;
        }
        if (item.name === "confirm") {
          return null;
        }
        if (item.name === "terms") {
          return null;
        }
        if (item.name === "privacy") {
          return null;
        }
        if (item.children) {
          return <Folder key={item.name} item={item} anchors={anchors} />;
        }
        return <File key={item.name} item={item} anchors={anchors} />;
      })}
    </ul>
  );
}

export default function Sidebar({
  directories,
  flatDirectories,
  fullDirectories,
  mdShow = true,
  headings = [],
}) {
  const config = useConfig();
  const anchors = useMemo(
    () =>
      headings
        .filter((v) => v.children && v.depth === 2 && v.type === "heading")
        .map((v) => v.value || "")
        .filter(Boolean),
    [headings]
  );
  const [hasScrolled, setHasScrolled] = useState(false);

  const onScroll = useCallback(() => {
    setHasScrolled(window.pageYOffset > 1);
  }, []);

  useEffect(() => {
    if (typeof window !== "undefined") {
      requestIdleCallback(onScroll);
      window.addEventListener("scroll", onScroll);
    }
    return () => {
      if (typeof window !== "undefined") {
        window.removeEventListener("scroll", onScroll);
      }
    };
  }, [onScroll]);

  const { menu } = useMenuContext();
  useEffect(() => {
    if (menu) {
      document.body.classList.add("overflow-hidden");
    } else {
      document.body.classList.remove("overflow-hidden");
    }
  }, [menu]);

  return (
    <aside
      className={cn(
        "fixed h-screen bg-white dark:bg-dark flex-shrink-0 w-full md:w-64 md:sticky z-20",
        menu ? "" : "hidden",
        mdShow ? "md:block" : ""
      )}
      style={{
        top: hasScrolled ? "4rem" : "6rem",
        height: hasScrolled ? "calc(100vh - 4rem)" : "calc(100vh - 6rem)",
      }}
    >
      <div className="w-full h-full p-4 pb-40 overflow-y-auto border-gray-200 sidebar dark:border-gray-900 md:pb-16">
        <div className="block mb-4 md:hidden">
          {config.customSearch ||
            (config.search ? <Search directories={flatDirectories} /> : null)}
        </div>
        <div className="hidden md:block">
          <Menu
            directories={directories}
            anchors={
              // When the viewport size is larger than `md`, hide the anchors in
              // the sidebar when `floatTOC` is enabled.
              config.floatTOC ? [] : anchors
            }
          />
        </div>
        <div className="md:hidden">
          <Menu
            directories={fullDirectories}
            anchors={
              // Always show the anchor links on mobile (`md`).
              anchors
            }
          />
          <ul>
            <li key="Enterprise">
              <a href={config.enterpriseLink} target="_blank" rel="noreferrer">
                Enterprise
              </a>
            </li>
          </ul>
        </div>
      </div>
    </aside>
  );
}
