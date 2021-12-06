import React, { useMemo, useState } from "react";
import { useRouter } from "next/router";
import "focus-visible";
import { SkipNavContent } from "@reach/skip-nav";
import { ThemeProvider } from "next-themes";
import cn from "classnames";

import Head from "./head";
import Navbar from "./navbar";
import Footer, { NavLinks } from "./footer";
import Theme from "./misc/theme";
import Sidebar from "./sidebar";
import ToC from "./toc";
import { ThemeConfigContext, useConfig } from "./config";
import { ActiveAnchor } from "./misc/active-anchor";
import defaultConfig from "./misc/default.config";
import { getFSRoute } from "./utils/get-fs-route";
import { MenuContext } from "./utils/menu-context";
import normalizePages from "./utils/normalize-pages";
import { getHeadings } from "./utils/get-headings";
import { getTitle } from "./utils/get-title";

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

function Body({ meta, toc, filepathWithName, navLinks, children }) {
  const config = useConfig();
  return (
    <React.Fragment>
      <SkipNavContent />
      {meta.headeronly ? (
        <div className="relative w-full overflow-x-hidden">{children}</div>
      ) : meta.full ? (
        <article className="relative w-full overflow-x-hidden nextra-content">
          {children}
        </article>
      ) : (
        <article className="relative flex w-full max-w-full min-w-0 px-6 pb-16 docs-container md:px-8">
          <main className="z-10 max-w-screen-md min-w-0 pt-4 mx-auto nextra-content">
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

const Layout = ({ filename, pageMap, meta, children }) => {
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

  const content = children.type();
  const filepath = route.slice(0, route.lastIndexOf("/") + 1);
  const filepathWithName = filepath + filename;
  const headings = getHeadings(content.props.children);
  const title = meta.title || getTitle(headings) || "Untitled";

  const isRTL = useMemo(() => {
    if (!config.i18n) return config.direction === "rtl" || null;
    const localeConfig = config.i18n.find((l) => l.locale === locale);
    return localeConfig && localeConfig.direction === "rtl";
  }, [config.i18n, locale]);

  const [menu, setMenu] = useState(false);

  if (activeType === "nav") {
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
              page: true,
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
                  mdShow={false}
                  headings={headings}
                />
                <Body
                  meta={meta}
                  filepathWithName={filepathWithName}
                  navLinks={null}
                >
                  {content}
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
                toc={<ToC headings={config.floatTOC ? headings : null} />}
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

export default (opts, config) => {
  const extendedConfig = Object.assign({}, defaultConfig, config);

  return (props) => {
    return (
      <ThemeConfigContext.Provider value={extendedConfig}>
        <ThemeProvider attribute="class">
          <Layout {...opts} {...props} />
        </ThemeProvider>
      </ThemeConfigContext.Provider>
    );
  };
};
