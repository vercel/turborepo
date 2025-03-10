import cn from "classnames";
import { TURBO_TEAM } from "./team";
import type { Author } from "./team";
import { Avatar } from "./avatar";

export function Authors({ authors }: { authors: Author[] }): JSX.Element {
  return (
    <div className="authors w-full border-b border-gray-400 border-opacity-20">
      <div
        className={cn(
          "mx-auto flex flex-wrap justify-center gap-7 py-8",
          authors.length > 4 && "max-w-3xl"
        )}
      >
        {authors.map((username) => (
          <Avatar key={username} {...TURBO_TEAM[username]} />
        ))}
      </div>
    </div>
  );
}
