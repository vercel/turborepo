import { Command } from "cmdk";
import Link from "next/link";
import { useEffect, useState } from "react";

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

  // async function handleSearch(e) {
  // }

  return (
    <>
      <Command label="Command Menu">
        <Command.Input
          onValueChange={(e) => {
            setValue(e);
          }}
          value={value}
        />
        <Command.List>
          <Command.Group heading="Results">
            <Command.Empty>No results found.</Command.Empty>
          </Command.Group>
        </Command.List>
      </Command>
      {results.map((result) => {
        return (
          // <Command.Item key={result.id}>
          <Result key={result.id} result={result} />
          // </Command.Item>
        );
      })}
    </>
  );
}

function Page() {
  return <CommandMenu />;
}

export default Page;
