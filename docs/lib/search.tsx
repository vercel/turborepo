import { useEffect, useState } from "react";

export const useResult = (result: any) => {
  const [finalData, setFinalData] = useState(null);

  useEffect(() => {
    async function fetchData() {
      const data = await result.data();
      setFinalData(data);
    }
    fetchData();
  }, [result]);

  return finalData;
};

export const usePageFindSearch = () => {
  useEffect(() => {
    async function loadPagefind() {
      if (typeof window.pagefind === "undefined") {
        try {
          window.pagefind = await import(
            // @ts-expect-error pagefind.js generated after build
            /* webpackIgnore: true */ "./pagefind/pagefind.js"
          );
        } catch (e) {
          window.pagefind = { search: () => ({ results: [] }) };
        }
      }
    }
    loadPagefind();
  }, []);
};

export const useSearchResults = (query: string) => {
  const [results, setResults] = useState();

  useEffect(() => {
    const thing = async () => {
      if (window.pagefind) {
        const search = await window.pagefind.search(query);
        setResults(search.results);
      }
    };
    thing();
  }, [query]);

  return results;
};
