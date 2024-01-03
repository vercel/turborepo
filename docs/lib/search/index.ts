import { useEffect, useState } from "react";
import { mockSearchData } from "../mock-search-data";
import type {
  PagefindSearchFragment,
  PagefindSearchResult,
  PagefindSearchResults,
} from "./search-types";

export const ignoredRoutes = ["/blog", "/terms", "/privacy", "/confirm"];

export const downrankedRoutes = [
  "/repo/docs/acknowledgements",
  // Deprecations
  "/repo/docs/core-concepts/scopes",
];

declare global {
  interface Window {
    pagefind?: {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any -- Not sure where this type comes from.
      search: (...args: any[]) => Promise<PagefindSearchResults>;
    };
  }
}

export const usePageFindSearch = () => {
  useEffect(() => {
    async function loadPagefind() {
      if (typeof window.pagefind === "undefined") {
        try {
          // eslint-disable-next-line @typescript-eslint/no-unsafe-assignment -- Not sure where to get this type.
          window.pagefind = await import(
            // @ts-expect-error -- Generated at buildtime
            // eslint-disable-next-line import/no-unresolved -- Generated at buildtime
            /* webpackIgnore: true */ "./pagefind/pagefind.js"
          );
        } catch (e) {
          if (process.env.NODE_ENV === "development") {
            window.pagefind = {
              search: () =>
                new Promise((resolve) => {
                  resolve({
                    results: mockSearchData,
                  } as unknown as PagefindSearchResults);
                }),
            };
            return;
          }

          window.pagefind = {
            search: () =>
              new Promise((resolve) => {
                resolve({ results: [] } as unknown as PagefindSearchResults);
              }),
          };
        }
      }
    }
    void loadPagefind();
  }, []);
};

export const useSearchResults = (query: string) => {
  const [results, setResults] = useState<PagefindSearchResults["results"]>();

  useEffect(() => {
    const handleSearch = async () => {
      if (window.pagefind) {
        const search = await window.pagefind.search(query);
        setResults(search.results);
      }
    };
    void handleSearch();
  }, [query]);

  return results;
};
export const useResult = (result: PagefindSearchResult) => {
  const [data, setData] = useState<PagefindSearchFragment | null>(null);

  useEffect(() => {
    async function fetchData() {
      const rawData = await result.data();
      setData(rawData);
    }
    void fetchData();
  }, [result]);

  return data;
};
