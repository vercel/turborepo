import { Avatar } from "./Avatar";
import cn from "classnames";
const team = {
  jaredpalmer: {
    name: "Jared Palmer",
    twitterUsername: "jaredpalmer",
    picture: "/images/people/jaredpalmer_headshot.jpeg",
  },
  gaspargarcia_: {
    name: "Gaspar Garcia",
    twitterUsername: "gaspargarcia_",
    picture: "/images/people/gaspargarcia_.jpeg",
  },
  becca__z: {
    name: "Becca Z.",
    twitterUsername: "becca__z",
    picture: "/images/people/becca__z.jpeg",
  },
  gsoltis: {
    name: "Greg Soltis",
    twitterUsername: "gsoltis",
    picture: "/images/people/gsoltis.jpeg",
  },
  nathanhammond: {
    name: "Nathan Hammond",
    twitterUsername: "nathanhammond",
    picture: "/images/people/nathanhammond.png",
  },
  tknickman: {
    name: "Tom Knickman",
    twitterUsername: "tknickman",
    picture: "/images/people/tknickman.jpeg",
  },
};

export function Authors({ authors }) {
  return (
    <div className="w-full border-b border-gray-400 authors border-opacity-20">
      <div
        className={cn(
          "flex flex-wrap justify-center py-8 mx-auto gap-7",
          authors.length > 4 && "max-w-3xl"
        )}
      >
        {authors.map((username) =>
          !!team[username] ? (
            <Avatar key={username} {...team[username]} />
          ) : (
            console.warning("no author found for", username) || null
          )
        )}
      </div>
    </div>
  );
}
