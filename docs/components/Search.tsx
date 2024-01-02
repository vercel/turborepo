import { useEffect, useRef, useState } from "react";
import Link from "next/link";
import { usePageFindSearch, useResult, useSearchResults } from "../lib/search";
import type { PagefindSearchResult } from "../lib/search-types";

function Result({ result }: { result: PagefindSearchResult }) {
  const data = useResult(result);

  if (!data) return <p className="text-gray-400 m-2">No results.</p>;

  // const formattedExcerpt = data.excerpt.replaceAll(
  //   "<mark>",
  //   '<mark className="bg-green-300"'
  // );

  return (
    <li className="mx-2 border-b border-gray-200 pb-2 dark:border-gray-700 dark:text-white text-gray-700">
      <Link
        className="hover:bg-blue-300/30 flex flex-col gap-2 p-2 px-3"
        href={data.url
          .replace("_next/static/chunks/pages/server/pages/", "")
          .replace(".html", "")}
      >
        <p className="text-lg font-semibold truncate">{data.meta.title}</p>
        <p
          className="line-clamp-3"
          dangerouslySetInnerHTML={{ __html: data.excerpt }}
        />
      </Link>
    </li>
  );
}

export function Search() {
  const [query, setQuery] = useState("");

  usePageFindSearch();
  const results = useSearchResults(query);

  const ref = useRef<HTMLInputElement | null>(null);

  const handleListener = (e: KeyboardEvent) => {
    if (e.key === "Escape" && document.activeElement === ref.current) {
      ref.current?.blur();
    }

    if (e.metaKey && e.key === "k") {
      if (document.activeElement === ref.current) {
        ref.current?.blur();
      } else {
        ref.current?.focus();
      }
    }
  };

  useEffect(() => {
    document.addEventListener("keydown", handleListener);

    return () => {
      document.removeEventListener("keydown", handleListener);
    };
  }, []);

  return (
    <div className="relative lg:w-60">
      <input
        className="p-2 px-3 rounded-lg text-sm w-full bg-gray-100 dark:bg-gray-900"
        onChange={(e) => {
          setQuery(e.target.value);
        }}
        placeholder="Search..."
        ref={ref}
        value={query}
      />
      {query.length > 0 && results ? (
        <ul className="border no-scrollbar border-gray-200 flex flex-col gap-1 bg-white text-gray-100 dark:border-neutral-800 dark:bg-neutral-900 absolute top-full z-20 mt-2 overflow-auto overscroll-contain rounded-xl py-2.5 shadow-xl max-h-[min(calc(50vh-11rem-env(safe-area-inset-bottom)),400px)] md:max-h-[min(calc(100vh-5rem-env(safe-area-inset-bottom)),400px)] inset-x-0 ltr:md:left-auto rtl:md:right-auto contrast-more:border contrast-more:border-gray-900 contrast-more:dark:border-gray-50 w-screen min-h-[100px] max-w-[min(calc(100vw-2rem),calc(100%+20rem))]">
          {results.map((result) => {
            return <Result key={result.id} result={result} />;
          })}
        </ul>
      ) : null}
    </div>
  );
}
