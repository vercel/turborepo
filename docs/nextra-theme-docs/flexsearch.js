import React, {
  memo,
  useCallback,
  useRef,
  useState,
  useEffect,
  Fragment,
} from "react";
import Router, { useRouter } from "next/router";
import cn from "classnames";
import Link from "next/link";
import FlexSearch from "flexsearch";
import { Transition } from "@headlessui/react";

import { useConfig } from "./config";
import renderComponent from "./utils/render-component";

const Item = ({ page, first, title, active, href, onHover, excerpt }) => {
  return (
    <>
      {first ? (
        <div className="mx-2.5 px-2.5 pb-1.5 mb-2 mt-6 first:mt-0 border-b font-semibold uppercase text-xs text-gray-500 border-gray-200 select-none dark:text-gray-300 dark:border-opacity-10">
          {page}
        </div>
      ) : null}
      <Link href={href}>
        <a className="block no-underline" onMouseMove={onHover}>
          <li className={cn({ active })}>
            <div className="font-semibold leading-5 dark:text-white">
              {title}
            </div>
            {excerpt ? (
              <div className="excerpt mt-1 text-gray-600 text-sm leading-[1.35rem] dark:text-gray-400">
                {excerpt}
              </div>
            ) : null}
          </li>
        </a>
      </Link>
    </>
  );
};

const MemoedStringWithMatchHighlights = memo(
  function StringWithMatchHighlights({ content, search }) {
    const splittedText = content.split("");
    const escapedSearch = search.trim().replace(/[|\\{}()[\]^$+*?.]/g, "\\$&");
    const regexp = RegExp(
      "(" + escapedSearch.split(" ").join("|") + ")",
      "ig"
    );
    let match;
    let id = 0;
    let index = 0;
    const res = [];

    while ((match = regexp.exec(content)) !== null) {
      res.push(
        <Fragment key={id++}>
          {splittedText.splice(0, match.index - index).join("")}
        </Fragment>
      );
      res.push(
        <span className="highlight" key={id++}>
          {splittedText.splice(0, regexp.lastIndex - match.index).join("")}
        </span>
      );
      index = regexp.lastIndex;
    }

    res.push(<Fragment key={id++}>{splittedText.join("")}</Fragment>);

    return res;
  }
);

// This can be global for better caching.
const indexes = {};

export default function Search() {
  const config = useConfig();
  const router = useRouter();
  const [loading, setLoading] = useState(false);
  const [show, setShow] = useState(false);
  const [search, setSearch] = useState("");
  const [active, setActive] = useState(0);
  const [results, setResults] = useState([]);
  const input = useRef(null);

  const doSearch = () => {
    if (!search) return;

    const localeCode = Router.locale || "default";
    const index = indexes[localeCode];

    if (!index) return;

    const pages = {};
    const results = []
      .concat(
        ...index
          .search(search, { enrich: true, limit: 10, suggest: true })
          .map((r) => r.result)
      )
      .map((r, i) => ({
        ...r,
        index: i,
        matchTitle:
          r.doc.content.indexOf(search) > r.doc.content.indexOf(" _NEXTRA_ "),
      }))
      .sort((a, b) => {
        if (a.matchTitle !== b.matchTitle) return a.matchTitle ? -1 : 1;
        if (a.doc.page !== b.doc.page) return a.doc.page > b.doc.page ? 1 : -1;
        return a.index - b.index;
      })
      .map((item) => {
        const firstItemOfPage = !pages[item.doc.page];
        pages[item.doc.page] = true;

        return {
          first: firstItemOfPage,
          route: item.doc.url,
          page: item.doc.page,
          title: (
            <MemoedStringWithMatchHighlights
              content={item.doc.title}
              search={search}
            />
          ),
          excerpt:
            item.doc.title !== item.doc.content ? (
              <MemoedStringWithMatchHighlights
                content={item.doc.content.replace(/ _NEXTRA_ .*$/, "")}
                search={search}
              />
            ) : null,
        };
      });

    setResults(results);
  };
  useEffect(doSearch, [search]);

  const handleKeyDown = useCallback(
    (e) => {
      switch (e.key) {
        case "ArrowDown": {
          e.preventDefault();
          if (active + 1 < results.length) {
            setActive(active + 1);
            const activeElement = document.querySelector(
              `.nextra-flexsearch ul > a:nth-of-type(${active + 2})`
            );
            if (activeElement && activeElement.scrollIntoView) {
              activeElement.scrollIntoView({
                behavior: "smooth",
                block: "nearest",
              });
            }
          }
          break;
        }
        case "ArrowUp": {
          e.preventDefault();
          if (active - 1 >= 0) {
            setActive(active - 1);
            const activeElement = document.querySelector(
              `.nextra-flexsearch ul > a:nth-of-type(${active})`
            );
            if (activeElement && activeElement.scrollIntoView) {
              activeElement.scrollIntoView({
                behavior: "smooth",
                block: "nearest",
              });
            }
          }
          break;
        }
        case "Enter": {
          router.push(results[active].route);
          break;
        }
      }
    },
    [active, results, router]
  );

  const load = async () => {
    const localeCode = Router.locale || "default";
    if (!indexes[localeCode] && !loading) {
      setLoading(true);
      const data = await (
        await fetch(`/_next/static/chunks/nextra-data-${localeCode}.json`)
      ).json();

      const index = new FlexSearch.Document({
        cache: 100,
        tokenize: "full",
        document: {
          id: "id",
          index: "content",
          store: ["title", "content", "url", "page"],
        },
        context: {
          resolution: 9,
          depth: 1,
          bidirectional: true,
        },
        filter: ["_NEXTRA_"],
      });

      for (let route in data) {
        for (let heading in data[route].data) {
          const [hash, text] = heading.split("#");
          const title = text || data[route].title;
          const url = route + (hash ? "#" + hash : "");

          const paragraphs = (data[route].data[heading] || "")
            .split("\n")
            .filter(Boolean);

          if (!paragraphs.length) {
            index.add({
              id: url,
              url: url,
              title,
              content: title,
              page: data[route].title,
            });
          }

          for (let i = 0; i < paragraphs.length; i++) {
            index.add({
              id: url + "_" + i,
              url: url,
              title: title,
              content: paragraphs[i] + (i === 0 ? " _NEXTRA_ " + title : ""),
              page: data[route].title,
            });
          }
        }
      }

      indexes[localeCode] = index;
      setLoading(false);
      setSearch((s) => (s ? s + " " : s)); // Trigger the effect
    }
  };

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

  const renderList = show && !!search;

  return (
    <div className="relative w-full nextra-search nextra-flexsearch md:w-64">
      {renderList && (
        <div className="z-10 search-overlay" onClick={() => setShow(false)} />
      )}
      <div className="relative flex items-center">
        <input
          onChange={(e) => {
            setSearch(e.target.value);
            setShow(true);
          }}
          className="block w-full px-3 py-2 leading-tight transition-colors rounded-lg appearance-none focus:outline-none focus:ring-1 focus:ring-gray-200 focus:bg-white hover:bg-opacity-5 dark:focus:bg-dark dark:focus:ring-gray-100 dark:focus:ring-opacity-20"
          type="search"
          placeholder={renderComponent(
            config.searchPlaceholder,
            {
              locale: router.locale,
            },
            true
          )}
          onKeyDown={handleKeyDown}
          onFocus={() => {
            load();
            setShow(true);
          }}
          onBlur={() => setShow(false)}
          ref={input}
          spellCheck={false}
        />
        {renderList ? null : (
          <div className="hidden sm:flex absolute inset-y-0 right-0 py-1.5 pr-1.5 select-none pointer-events-none">
            <kbd className="inline-flex items-center px-2 font-mono text-sm font-medium text-gray-400 bg-white border rounded dark:bg-dark dark:bg-opacity-50 dark:text-gray-500 dark:border-gray-100 dark:border-opacity-20">
              /
            </kbd>
          </div>
        )}
      </div>
      <Transition
        show={renderList}
        as={React.Fragment}
        leave="transition duration-100"
        leaveFrom="opacity-100"
        leaveTo="opacity-0"
      >
        <ul className="absolute z-20 p-0 m-0 mt-2 top-full py-2.5">
          {loading ? (
            <span className="flex justify-center p-8 text-sm text-center text-gray-400 select-none">
              <svg
                className="w-5 h-5 mr-2 -ml-1 text-gray-400 animate-spin"
                xmlns="http://www.w3.org/2000/svg"
                fill="none"
                viewBox="0 0 24 24"
              >
                <circle
                  className="opacity-25"
                  cx="12"
                  cy="12"
                  r="10"
                  stroke="currentColor"
                  strokeWidth="4"
                ></circle>
                <path
                  className="opacity-75"
                  fill="currentColor"
                  d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z"
                ></path>
              </svg>
              <span>Loading...</span>
            </span>
          ) : results.length === 0 ? (
            renderComponent(config.unstable_searchResultEmpty, {
              locale: router.locale,
            })
          ) : (
            results.map((res, i) => {
              return (
                <Item
                  first={res.first}
                  key={`search-item-${i}`}
                  page={res.page}
                  title={res.title}
                  href={res.route}
                  excerpt={res.excerpt}
                  active={i === active}
                  onHover={() => setActive(i)}
                />
              );
            })
          )}
        </ul>
      </Transition>
    </div>
  );
}
