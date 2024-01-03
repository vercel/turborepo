import cn from "classnames";
import { TURBO_TEAM } from "../content/team";
import type { Author } from "../content/team";
import { Avatar } from "./Avatar";

export function Authors({ authors }: { authors: Author[] }) {
  // eslint-disable-next-line @typescript-eslint/no-unnecessary-condition -- This does what is is meant to.
  const validAuthors = authors.filter((author) => TURBO_TEAM[author]);
  return (
    <div className="w-full border-b border-gray-400 authors border-opacity-20">
      <div
        className={cn(
          "flex flex-wrap justify-center py-8 mx-auto gap-7",
          authors.length > 4 && "max-w-3xl"
        )}
      >
        {validAuthors.map((username) => (
          <Avatar key={username} {...TURBO_TEAM[username]} />
        ))}
      </div>
    </div>
  );
}
