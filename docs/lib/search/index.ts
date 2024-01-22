import { useEffect, useState } from "react";
import { mockSearchData } from "../mock-search-data";
import type {
  PagefindSearchFragment,
  PagefindSearchResult,
  PagefindSearchResults,
} from "./search-types";

// Default weight is 1.
// Recommended values (Acceptable to use different values as needed):
// Uprank: 1.2
// Downrank .2
// Ignore a route with 0.

export const weightedRoutes: [string, number][] = [
  ["/repo/docs/reference/configuration", 4],
  ["/repo/docs/reference/command-line-reference", 4],
  ["/repo/docs/core-concepts/caching", 1.2],
  ["/repo/docs/handbook/linting", 0.8],
  ["/repo/docs/handbook/linting/eslint", 0.8],
  ["/repo/docs/acknowledgements", 0.2],
  ["/repo/docs/core-concepts/caching/to-cache-or-not-to-cache", 0.2],
  // Deprecations
  ["/repo/docs/core-concepts/scopes", 0.2],
  // Ignored pages
  ["/blog", 0],
  ["/terms", 0],
  ["/privacy", 0],
  ["/confirm", 0],
  ["/governance", 0],
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
        // Filter away distant matches.
        setResults(search.results.filter((result) => result.score > 0.01));
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
