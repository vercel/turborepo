import Link from "next/link";
import { useEffect, useState } from "react";

function Result({ result }) {
  const [data, setData] = useState(null);

  console.log(data);

  useEffect(() => {
    async function fetchData() {
      const data = await result.data();
      setData(data);
    }
    fetchData();
  }, [result]);

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

export function CommandMenu() {
  const [value, setValue] = useState("");
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

  useEffect(() => {
    const thing = async () => {
      if (window.pagefind) {
        const search = await window.pagefind.search(value);
        setResults(search.results);
      }
    };
    thing();
  }, [value]);

  return (
    <>
      <input
        onChange={(e) => {
          setValue(e.target.value);
        }}
        value={value}
      />
      <div className="flex flex-col gap-6">
        {results.map((result) => {
          return <Result key={result.id} result={result} />;
        })}
      </div>
    </>
  );
}

function Page() {
  return <CommandMenu />;
}

export default Page;
