import { cn } from "@/lib/utils";
import { TURBO_TEAM } from "./team";
import type { Author } from "./team";
import { Avatar } from "./avatar";

export function Authors({ authors }: { authors: Array<Author> }) {
  return (
    <div className="authors w-full border-b border-opacity-20">
      <div
        className={cn(
          "mx-auto flex flex-wrap gap-7 py-8",
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
