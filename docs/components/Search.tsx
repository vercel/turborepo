import { useEffect, useRef, useState } from "react";

export function Search() {
  const [query, setQuery] = useState("");

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
      {query.length > 0 ? <p>hi</p> : null}
    </div>
  );
}
