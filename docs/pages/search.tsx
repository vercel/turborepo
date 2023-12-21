import Link from "next/link";
import { useState, useEffect } from "react";

export default function SearchPage() {
  const [query, setQuery] = useState("");
  const [results, setResults] = useState([]);

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

  async function handleSearch() {
    if (window.pagefind) {
      const search = await window.pagefind.search(query);
      setResults(search.results);
    }
  }

  return (
    <div>
      <input
        onChange={(e) => {
          setQuery(e.target.value);
        }}
        onInput={handleSearch}
        placeholder="Search..."
        type="text"
        value={query}
      />
      <div className="flex gap-4 flex-col" id="results">
        {results.map((result, index) => (
          <Result key={result.id} result={result} />
        ))}
      </div>
    </div>
  );
}

function Result({ result }) {
  const [data, setData] = useState(null);

  useEffect(() => {
    async function fetchData() {
      const data = await result.data();
      setData(data);
    }
    fetchData();
  }, [result]);

  if (!data) return null;

  return (
    <Link className="hover:bg-gray-300" href={data.url}>
      <h3>{data.meta.title}</h3>
      <p>{data.excerpt}</p>
    </Link>
  );
}
