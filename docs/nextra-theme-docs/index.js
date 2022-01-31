import React, { useMemo, useState } from "react";
import format from "date-fns/format";
import { useRouter } from "next/router";
import "focus-visible";
import { SkipNavContent } from "@reach/skip-nav";
import { ThemeProvider } from "next-themes";
import cn from "classnames";
import Head from "./head";
import Navbar from "./navbar";
import Footer, { NavLinks } from "./footer";
import { MDXTheme } from "./misc/theme";
import Sidebar from "./sidebar";
import ToC from "./toc";
import { ThemeConfigContext, useConfig } from "./config";
import { ActiveAnchor } from "./misc/active-anchor";
import defaultConfig from "./misc/default.config";
import { getFSRoute } from "./utils/get-fs-route";
import { MenuContext } from "./utils/menu-context";
import normalizePages from "./utils/normalize-pages";
import traverse from "./utils/traverse";
import sortDate from "./utils/sort-date";
import Link from "next/link";
import { Footer as FooterMain } from "../components/Footer";
import { Avatar } from "../components/Avatar";
import { formatDistanceToNow } from "date-fns";
import renderComponent from "./utils/render-component";
function useDirectoryInfo(pageMap) {
  const { locale, defaultLocale, asPath } = useRouter();

  return useMemo(() => {
    const fsPath = getFSRoute(asPath, locale).split("#")[0];
    return normalizePages({
      list: pageMap,
      locale,
      defaultLocale,
      route: fsPath,
    });
  }, [pageMap, locale, defaultLocale, asPath]);
}

function Body({ meta, toc, filepathWithName, navLinks, children, postList }) {
  const config = useConfig();
  return (
    <React.Fragment>
      <SkipNavContent />
      {meta.headeronly ? (
        meta.container ? (
          <div className="relative w-full mx-auto overflow-x-hidden">
            <article className="pb-24">
              <main className="z-10 max-w-screen-md min-w-0 px-6 pt-8 mx-auto">
                <MDXTheme>{children}</MDXTheme>
              </main>
            </article>
            <FooterMain />
          </div>
        ) : (
          <div className="relative w-full overflow-x-hidden">{children}</div>
        )
      ) : postList ? (
        <div className="relative w-full overflow-x-hidden">
          <div className="pb-24">
            <div className="px-6 py-8 mx-auto border-b dark:border-gray-800">
              <h1 className="max-w-screen-lg pt-2 pb-8 mx-auto text-4xl font-bold leading-tight text-center lg:text-5xl">
                Blog
              </h1>
              <div className="flex items-center justify-center mx-auto ">
                The latest updates and releases from the Turborepo team at
                Vercel.
              </div>
            </div>
            <main className="z-10 max-w-screen-md min-w-0 px-6 pt-8 mx-auto">
              {postList}
            </main>
          </div>
          <FooterMain />
        </div>
      ) : meta.full ? (
        <article className="relative w-full overflow-x-hidden nextra-content">
          {children}
        </article>
      ) : meta.type === "post" ? (
        <div className="relative w-full mx-auto overflow-x-hidden">
          <article className="pb-24">
            <div className="px-6 py-8 mx-auto space-y-8 text-center border-b dark:border-gray-800">
              <h1 className="max-w-screen-lg pt-2 mx-auto text-4xl font-bold leading-tight lg:text-5xl">
                {meta.title}
              </h1>
              <div className="text-gray-400 dark:text-gray-500">
                {format(new Date(meta.date), "MMMM do, yyyy")} (
                {formatDistanceToNow(new Date(meta.date), {
                  includeSeconds: false,
                  addSuffix: true,
                })}
                )
              </div>
              <div className="flex items-center justify-center mx-auto ">
                {config.authors
                  ? renderComponent(config.authors, { authors: meta.authors })
                  : null}
              </div>
            </div>
            <main className="z-10 max-w-screen-md min-w-0 px-6 pt-8 mx-auto">
              <MDXTheme>{children}</MDXTheme>
            </main>
          </article>
          <FooterMain />
        </div>
      ) : (
        <article className="relative flex w-full max-w-full min-w-0 px-6 pb-16 docs-container md:px-8">
          <main className="z-10 max-w-screen-md min-w-0 pt-4 mx-auto nextra-content">
            <MDXTheme>{children}</MDXTheme>
            <Footer config={config} filepathWithName={filepathWithName}>
              {navLinks}
            </Footer>
          </main>
          {toc}
        </article>
      )}
    </React.Fragment>
  );
}

const Layout = ({
  filename,
  pageMap,
  meta,
  route: _route,
  children,
  headings,
  titleText,
}) => {
  const { route, locale } = useRouter();
  const config = useConfig();

  const {
    activeType,
    activeIndex,
    // pageDirectories,
    flatPageDirectories,
    docsDirectories,
    flatDirectories,
    flatDocsDirectories,
    directories,
  } = useDirectoryInfo(pageMap);

  const filepath = route.slice(0, route.lastIndexOf("/") + 1);
  const filepathWithName = filepath + filename;
  const title = meta.title || titleText || "Untitled";

  // gather info for tag/posts pages
  let posts = null;
  let navPages = [];
  const type = meta.type || "page";
  // console.log(pageMap);
  // This only renders once per page
  if (type === "posts" || type === "tag" || type === "page") {
    posts = [];
    // let's get all posts
    traverse(pageMap, (page) => {
      if (
        page.frontMatter &&
        ["page", "posts"].includes(page.frontMatter.type)
      ) {
        if (page.route === route) {
          navPages.push({ ...page, active: true });
        } else {
          navPages.push(page);
        }
      }
      if (page.children) return;
      if (page.name.startsWith("_")) return;
      // if (
      //   type === "posts" &&
      //   !page.route.startsWith(route === "/posts" ? route : route + "/")
      // )
      //   return;
      if (page && page.frontMatter && page.frontMatter.type === "post") {
        posts.push(page);
      }
    });
    posts = posts.sort(sortDate);
    navPages = navPages.sort(sortDate);
  }

  // back button
  let back = null;
  if (type !== "post") {
    back = null;
  } else {
    const parentPages = [];
    traverse(pageMap, (page) => {
      if (
        route !== page.route &&
        (route + "/").startsWith(page.route === "/" ? "/" : page.route + "/")
      ) {
        parentPages.push(page);
      }
    });
    const parentPage = parentPages
      .reverse()
      .find((page) => page.frontMatter && page.frontMatter.type === "posts");
    if (parentPage) {
      back = parentPage.route;
    }
  }

  const postList = posts ? (
    <ul className="pb-24 space-y-10 ">
      {posts.map((post) => {
        const postTitle =
          (post.frontMatter ? post.frontMatter.title : null) || post.name;
        const postDate = post.frontMatter ? (
          <time className="post-item-date">
            {format(new Date(post.frontMatter.date), "MMMM do, yyyy")}
          </time>
        ) : null;
        const postDescription =
          post.frontMatter && post.frontMatter.description ? (
            <p className="post-item-desc">
              {post.frontMatter.description}
              {config.readMore ? (
                <Link href={post.route}>
                  <a className="post-item-more">{config.readMore}</a>
                </Link>
              ) : null}
            </p>
          ) : null;

        return (
          <div key={post.route} className="post-item">
            <h3>
              <Link href={post.route}>
                <a className="font-bold text-current no-underline post-item-title">
                  {postTitle}
                </a>
              </Link>
            </h3>
            {postDescription}
            {postDate}
          </div>
        );
      })}
    </ul>
  ) : null;

  const isRTL = useMemo(() => {
    if (!config.i18n) return config.direction === "rtl" || null;
    const localeConfig = config.i18n.find((l) => l.locale === locale);
    return localeConfig && localeConfig.direction === "rtl";
  }, [config.i18n, locale]);

  const [menu, setMenu] = useState(false);

  if (
    activeType === "nav" ||
    meta.headeronly ||
    meta.type === "post" ||
    meta.type === "posts"
  ) {
    return (
      <React.Fragment>
        <Head title={title} locale={locale} meta={meta} />
        <MenuContext.Provider
          value={{
            menu,
            setMenu,
            defaultMenuCollapsed: !!config.defaultMenuCollapsed,
          }}
        >
          <div
            className={cn("nextra-container main-container flex flex-col", {
              rtl: isRTL,
            })}
          >
            <Navbar
              isRTL={isRTL}
              flatDirectories={flatDirectories}
              flatPageDirectories={flatPageDirectories}
            />
            <ActiveAnchor>
              <div className="flex flex-1 h-full">
                <Sidebar
                  directories={flatPageDirectories}
                  flatDirectories={flatDirectories}
                  fullDirectories={directories}
                  headings={headings}
                  mdShow={false}
                />
                <Body
                  meta={meta}
                  filepathWithName={filepathWithName}
                  navLinks={
                    meta.type === "post" ? (
                      <NavLinks
                        flatDirectories={flatDocsDirectories}
                        currentIndex={activeIndex}
                        isRTL={isRTL}
                      />
                    ) : null
                  }
                  postList={postList}
                >
                  {children}
                </Body>
              </div>
            </ActiveAnchor>
          </div>
        </MenuContext.Provider>
      </React.Fragment>
    );
  }

  // Docs layout
  return (
    <React.Fragment>
      <Head title={title} locale={locale} meta={meta} />
      <MenuContext.Provider
        value={{
          menu,
          setMenu,
          defaultMenuCollapsed: !!config.defaultMenuCollapsed,
        }}
      >
        <div
          className={cn("nextra-container main-container flex flex-col", {
            rtl: isRTL,
          })}
        >
          <Navbar
            isRTL={isRTL}
            flatDirectories={flatDirectories}
            flatPageDirectories={flatPageDirectories}
          />
          <ActiveAnchor>
            <div className="flex flex-1 h-full">
              <Sidebar
                directories={docsDirectories}
                flatDirectories={flatDirectories}
                fullDirectories={directories}
                headings={headings}
              />
              <Body
                meta={meta}
                filepathWithName={filepathWithName}
                toc={
                  <ToC
                    headings={config.floatTOC ? headings : null}
                    filepathWithName={filepathWithName}
                  />
                }
                navLinks={
                  <NavLinks
                    flatDirectories={flatDocsDirectories}
                    currentIndex={activeIndex}
                    isRTL={isRTL}
                  />
                }
              >
                {children}
              </Body>
            </div>
          </ActiveAnchor>
        </div>
      </MenuContext.Provider>
    </React.Fragment>
  );
};

export default function NextraThemeLayout(opts, config) {
  const extendedConfig = Object.assign({}, defaultConfig, config);

  return function NextraLayout(props) {
    return (
      <ThemeConfigContext.Provider value={extendedConfig}>
        <ThemeProvider attribute="class" defaultTheme="dark">
          <Layout {...opts} {...props} />
        </ThemeProvider>
      </ThemeConfigContext.Provider>
    );
  };
}
