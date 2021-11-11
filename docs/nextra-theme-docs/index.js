import React, { useMemo, useState } from "react";
import { useRouter } from "next/router";
import "focus-visible";
import { SkipNavContent } from "@reach/skip-nav";
import { ThemeProvider } from "next-themes";
import innerText from "react-innertext";
import cn from "classnames";

import normalizePages from "./utils/normalize-pages";

import Head from "./head";
import Navbar from "./navbar";
import Footer, { NavLinks } from "./footer";
import Theme from "./misc/theme";
import Sidebar from "./sidebar";
import ToC from "./toc";
import { ActiveAnchor } from "./misc/active-anchor";
import defaultConfig from "./misc/default.config";
import { getFSRoute } from "./utils/get-fs-route";
import { MenuContext } from "./utils/menu-context";

const titleType = ["h1", "h2", "h3", "h4", "h5", "h6"];

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

function Body({ meta, config, toc, filepathWithName, navLinks, children }) {
  return (
    <React.Fragment>
      <SkipNavContent />
      {meta.full ? (
        <article className="relative w-full overflow-x-hidden">
          {children}
        </article>
      ) : (
        <article className="relative flex w-full max-w-full min-w-0 px-6 pb-16 docs-container md:px-8">
          <main className="z-10 max-w-screen-md min-w-0 pt-4 mx-auto">
            <Theme>{children}</Theme>
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

const Layout = ({ filename, config: _config, pageMap, meta, children }) => {
  const { route, locale } = useRouter();

  // @TODO: config should be in a context.
  const config = Object.assign({}, defaultConfig, _config);

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
  const titles = React.Children.toArray(children).filter(
    (child) => child.props && titleType.includes(child.props.mdxType)
  );
  const titleEl = titles.find(
    (child) => child.props && child.props.mdxType === "h1"
  );
  const title =
    meta.title || (titleEl ? innerText(titleEl.props.children) : "Untitled");
  const anchors = titles
    .filter(
      (child) =>
        child.props && (config.floatTOC || child.props.mdxType === "h2")
    )
    .map((child) => child.props.children);

  const isRTL = useMemo(() => {
    if (!config.i18n) return config.direction === "rtl" || null;
    const localeConfig = config.i18n.find((l) => l.locale === locale);
    return localeConfig && localeConfig.direction === "rtl";
  }, [config.i18n, locale]);

  const [menu, setMenu] = useState(false);

  if (activeType === "nav") {
    return (
      <React.Fragment>
        <Head config={config} title={title} locale={locale} meta={meta} />
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
              page: true,
            })}
          >
            <Navbar
              config={config}
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
                  mdShow={false}
                  config={config}
                />
                <Body
                  meta={meta}
                  config={config}
                  filepathWithName={filepathWithName}
                  navLinks={null}
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
      <Head config={config} title={title} locale={locale} meta={meta} />
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
            config={config}
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
                anchors={config.floatTOC ? [] : anchors}
                config={config}
              />
              <Body
                meta={meta}
                config={config}
                filepathWithName={filepathWithName}
                toc={<ToC titles={config.floatTOC ? titles : null} />}
                navLinks={
                  <NavLinks
                    flatDirectories={flatDocsDirectories}
                    currentIndex={activeIndex}
                    config={config}
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

export default (opts, config) => (props) => {
  return (
    <ThemeProvider attribute="class">
      <Layout config={config} {...opts} {...props} />
    </ThemeProvider>
  );
};
