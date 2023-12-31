import { useEffect, useRef, useState } from "react";
import Link from "next/link";
import { usePageFindSearch, useResult, useSearchResults } from "../lib/search";

function Result({ result }) {
  // const [data, setData] = useState(null);

  const data = useResult(result);

  if (!data) return null;

  return (
    <Link
      className="hover:bg-gray-300 flex flex-col gap-2"
      href={data.url
        .replace("_next/static/chunks/pages/server/pages/", "")
        .replace(".html", "")}
    >
      <h3 className="text-lg font-bold">{data.meta.title}</h3>
      <p>{data.excerpt}</p>
    </Link>
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
    <div className="hidden relative md:block">
      <input
        className="p-2 px-3 rounded-lg text-sm w-60  bg-gray-100 dark:bg-gray-900"
        onChange={(e) => {
          setQuery(e.target.value);
        }}
        placeholder="Search documentation..."
        ref={ref}
        value={query}
      />
      {query.length > 0 && results
        ? results.map((result) => {
            return <Result key={result.id} result={result} />;
          })
        : null}
    </div>
  );
}
