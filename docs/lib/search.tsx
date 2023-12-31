import { useEffect, useState } from "react";
import type {
  PagefindSearchFragment,
  PagefindSearchResult,
  PagefindSearchResults,
} from "./search-types";

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
    const thing = async () => {
      if (window.pagefind) {
        const search = await window.pagefind.search(query);
        setResults(search.results);
      }
    };
    void thing();
  }, [query]);

  return results;
};
export const useResult = (result: PagefindSearchResult) => {
  const [finalData, setFinalData] = useState<PagefindSearchFragment | null>(
    null
  );

  useEffect(() => {
    async function fetchData() {
      const data = await result.data();
      setFinalData(data);
    }
    void fetchData();
  }, [result]);

  return finalData;
};
