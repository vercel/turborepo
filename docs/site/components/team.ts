export const TURBO_TEAM = {
  jaredpalmer: {
    name: "Jared Palmer",
    picture: "/images/people/jaredpalmer.jpeg",
    xUsername: "jaredpalmer",
  },
  gaspargarcia: {
    name: "Gaspar Garcia",
    picture: "/images/people/gaspargarcia.jpeg",
    xUsername: "gaspargarcia_",
  },
  becca__z: {
    name: "Becca Z.",
    picture: "/images/people/becca__z.jpeg",
    xUsername: "becca__z",
  },
  gregsoltis: {
    name: "Greg Soltis",
    picture: "/images/people/gregsoltis.jpeg",
    xUsername: "gsoltis",
  },
  nathanhammond: {
    name: "Nathan Hammond",
    picture: "/images/people/nathanhammond.png",
  },
  tomknickman: {
    name: "Tom Knickman",
    picture: "/images/people/tomknickman.jpeg",
    xUsername: "tknickman",
  },
  mehulkar: {
    name: "Mehul Kar",
    picture: "/images/people/mehulkar.jpeg",
    xUsername: "mehulkar",
  },
  mattpocock: {
    name: "Matt Pocock",
    picture: "/images/people/mattpocock.jpeg",
    xUsername: "mattpocockuk",
  },
  tobiaskoppers: {
    name: "Tobias Koppers",
    picture: "/images/people/tobiaskoppers-avatar.jpg",
    xUsername: "wSokra",
  },
  alexkirsz: {
    name: "Alex Kirszenberg",
    picture: "/images/people/alexkirsz.jpg",
    xUsername: "alexkirsz",
  },
  anthonyshew: {
    name: "Anthony Shew",
    picture: "/images/people/anthonyshew.jpg",
    xUsername: "anthonysheww",
  },
  nicholasyang: {
    name: "Nicholas Yang",
    picture: "/images/people/nicholasyang.png",
    xUsername: "nicholaslyang",
  },
  chrisolszewski: {
    name: "Chris Olszewski",
    picture: "/images/people/chrisolszewski.jpg",
  },
  alexanderlyon: {
    name: "Alexander Lyon",
    picture: "/images/people/alexanderlyon.jpg",
    xUsername: "_arlyon",
  },
  dimitrimitropoulos: {
    name: "Dimitri Mitropoulos",
    picture: "/images/people/dimitrimitropoulos.jpg",
  },
} as const;

export type Author = keyof typeof TURBO_TEAM;
export interface AuthorDetails {
  name: string;
  picture: string;
  xUsername?: string;
}
