import { Avatar } from "./Avatar";
import cn from "classnames";
import TURBO_TEAM from "../content/team";
import type { Author } from "../content/team";

export function Authors({ authors }: { authors: Array<Author> }) {
  return (
    <div className="w-full border-b border-gray-400 authors border-opacity-20">
      <div
        className={cn(
          "flex flex-wrap justify-center py-8 mx-auto gap-7",
          authors.length > 4 && "max-w-3xl"
        )}
      >
        {authors.map((username) =>
          !!TURBO_TEAM[username] ? (
            <Avatar key={username} {...TURBO_TEAM[username]} />
          ) : (
            console.warn("no author found for: ", username)
          )
        )}
      </div>
    </div>
  );
}
