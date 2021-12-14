import React, {
  useMemo,
  useCallback,
  useRef,
  useState,
  useEffect,
} from "react";
import { matchSorter } from "match-sorter";
import cn from "classnames";
import { useRouter } from "next/router";
import Link from "next/link";
import useMounted from "./utils/use-mounted";

const Item = ({ title, active, href, onMouseOver, search }) => {
  const highlight = title.toLowerCase().indexOf(search.toLowerCase());

  return (
    <Link href={href}>
      <a className="block no-underline" onMouseOver={onMouseOver}>
        <li className={cn("p-2", { active })}>
          {title.substring(0, highlight)}
          <span className="highlight">
            {title.substring(highlight, highlight + search.length)}
          </span>
          {title.substring(highlight + search.length)}
        </li>
      </a>
    </Link>
  );
};

const UP = true;
const DOWN = false;

const Search = ({ directories = [] }) => {
  const router = useRouter();
  const [show, setShow] = useState(false);
  const [search, setSearch] = useState("");
  const [active, setActive] = useState(0);
  const input = useRef(null);
  const isMounted = useMounted();
  const results = useMemo(() => {
    if (!search) return [];

    // Will need to scrape all the headers from each page and search through them here
    // (similar to what we already do to render the hash links in sidebar)
    // We could also try to search the entire string text from each page
    return matchSorter(directories, search, { keys: ["title"] });
  }, [search]);

  const moveActiveItem = (up) => {
    const position = active + (up ? -1 : 1);
    const { length } = results;

    // Modulo instead of remainder,
    // see https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Operators/Remainder
    const next = (position + length) % length;
    setActive(next);
  };

  const handleKeyDown = useCallback(
    (e) => {
      const { key, ctrlKey } = e;

      if ((ctrlKey && key === "n") || key === "ArrowDown") {
        e.preventDefault();
        moveActiveItem(DOWN);
      }

      if ((ctrlKey && key === "p") || key === "ArrowUp") {
        e.preventDefault();
        moveActiveItem(UP);
      }

      if (key === "Enter") {
        router.push(results[active].route);
      }
    },
    [active, results, router]
  );

  useEffect(() => {
    setActive(0);
  }, [search]);

  useEffect(() => {
    const inputs = ["input", "select", "button", "textarea"];

    const down = (e) => {
      if (
        document.activeElement &&
        inputs.indexOf(document.activeElement.tagName.toLowerCase()) === -1
      ) {
        if (e.key === "/") {
          e.preventDefault();
          input.current.focus();
        } else if (e.key === "Escape") {
          setShow(false);
        }
      }
    };

    window.addEventListener("keydown", down);
    return () => window.removeEventListener("keydown", down);
  }, []);

  const renderList = show && results.length > 0;

  return (
    <div className="relative w-full nextra-search md:w-64">
      {renderList && (
        <div className="z-10 search-overlay" onClick={() => setShow(false)} />
      )}

      <div className="relative flex items-center">
        <div className="absolute inset-y-0 left-0 flex items-center pl-3 pointer-events-none">
          <svg
            xmlns="http://www.w3.org/2000/svg"
            className="w-5 h-5 text-gray-400 dark:text-gray-600"
            fill="none"
            viewBox="0 0 24 24"
            stroke="currentColor"
            aria-hidden="true"
          >
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth={2}
              d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z"
            />
          </svg>
        </div>
        <input
          onChange={(e) => {
            setSearch(e.target.value);
            setShow(true);
          }}
          className="block w-full py-2 pl-10 leading-tight border rounded-md appearance-none focus:outline-none focus:ring"
          type="search"
          placeholder="Search docs..."
          onKeyDown={handleKeyDown}
          onFocus={() => setShow(true)}
          onBlur={() => {
            setTimeout(() => {
              if (isMounted) {
                setShow(false);
              }
            }, 300);
          }}
          ref={input}
          spellCheck={false}
        />
        {show ? null : (
          <div className="hidden sm:flex absolute inset-y-0 right-0 py-1.5 pr-1.5">
            <kbd className="inline-flex items-center px-2 font-sans text-sm font-medium text-gray-400 border rounded dark:text-gray-800 dark:border-gray-800">
              /
            </kbd>
          </div>
        )}
      </div>
      {renderList && (
        <ul className="absolute left-0 z-20 w-full p-0 m-0 mt-1 list-none border divide-y rounded-md shadow-md md:right-0 top-100 md:w-auto">
          {results.map((res, i) => {
            return (
              <Item
                key={`search-item-${i}`}
                title={res.title}
                href={res.route}
                active={i === active}
                search={search}
                onMouseOver={() => setActive(i)}
              />
            );
          })}
        </ul>
      )}
    </div>
  );
};

export default Search;
