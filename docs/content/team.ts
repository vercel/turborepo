const TURBO_TEAM: Record<string, AuthorDetails> = {
  jaredpalmer: {
    name: "Jared Palmer",
    twitterUsername: "jaredpalmer",
    picture: "/images/people/jaredpalmer.jpeg",
  },
  gaspargarcia: {
    name: "Gaspar Garcia",
    twitterUsername: "gaspargarcia_",
    picture: "/images/people/gaspargarcia.jpeg",
  },
  becca__z: {
    name: "Becca Z.",
    twitterUsername: "becca__z",
    picture: "/images/people/becca__z.jpeg",
  },
  gregsoltis: {
    name: "Greg Soltis",
    twitterUsername: "gsoltis",
    picture: "/images/people/gregsoltis.jpeg",
  },
  nathanhammond: {
    name: "Nathan Hammond",
    twitterUsername: "nathanhammond",
    picture: "/images/people/nathanhammond.png",
  },
  tomknickman: {
    name: "Tom Knickman",
    twitterUsername: "tknickman",
    picture: "/images/people/tomknickman.jpeg",
  },
  mehulkar: {
    name: "Mehul Kar",
    twitterUsername: "mehulkar",
    picture: "/images/people/mehulkar.jpeg",
  },
  mattpocock: {
    name: "Matt Pocock",
    twitterUsername: "mattpocockuk",
    picture: "/images/people/mattpocock.jpeg",
  },
  tobiaskoppers: {
    name: "Tobias Koppers",
    twitterUsername: "wSokra",
    picture: "/images/people/tobiaskoppers-avatar.jpg",
  },
  alexkirsz: {
    name: "Alex Kirszenberg",
    twitterUsername: "alexkirsz",
    picture: "/images/people/alexkirsz.jpg",
  },
  anthonyshew: {
    name: "Anthony Schew",
    twitterUsername: "anthonyShewDev",
    picture: "/images/people/anthonyshew.png",
  },
  nicholasyang: {
    name: "Nicholas Yang",
    twitterUsername: "nicholaslyang",
    picture: "/images/people/nicholasyang.png",
  },
  chrisolszewski: {
    name: "Chris Olszewski",
    picture: "/images/people/chrisolszewski.jpg",
  },
};

export type Author = keyof typeof TURBO_TEAM;
export type AuthorDetails = {
  name: string;
  twitterUsername?: string;
  picture: string;
};

export default TURBO_TEAM;
