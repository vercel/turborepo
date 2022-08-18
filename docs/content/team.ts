const TURBO_TEAM = {
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
  mehulkar: {
    name: "Mehul Kar",
    twitterUsername: "mehulkar",
    picture: "/images/people/mehulkar.jpeg",
  },
};

export type Author = keyof typeof TURBO_TEAM;
export type AuthorDetails = typeof TURBO_TEAM[Author];

export default TURBO_TEAM;
