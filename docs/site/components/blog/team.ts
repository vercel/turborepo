import jaredpalmer from "../../public/images/people/jaredpalmer.jpeg";
import gaspargarcia from "../../public/images/people/gaspargarcia.jpeg";
import becca__z from "../../public/images/people/becca__z.jpeg";
import gregsoltis from "../../public/images/people/gregsoltis.jpeg";
import nathanhammond from "../../public/images/people/nathanhammond.png";
import tomknickman from "../../public/images/people/tomknickman.jpeg";
import mehulkar from "../../public/images/people/mehulkar.jpeg";
import mattpocock from "../../public/images/people/mattpocock.jpeg";
import tobiaskoppers from "../../public/images/people/tobiaskoppers-avatar.jpg";
import alexkirsz from "../../public/images/people/alexkirsz.jpg";
import anthonyshew from "../../public/images/people/anthonyshew.jpg";
import nicholasyang from "../../public/images/people/nicholasyang.png";
import chrisolszewski from "../../public/images/people/chrisolszewski.jpg";
import alexanderlyon from "../../public/images/people/alexanderlyon.jpg";
import dimitrimitropoulos from "../../public/images/people/dimitrimitropoulos.jpg";

export const TURBO_TEAM = {
  jaredpalmer: {
    name: "Jared Palmer",
    picture: jaredpalmer,
    xUsername: "jaredpalmer"
  },
  gaspargarcia: {
    name: "Gaspar Garcia",
    picture: gaspargarcia,
    xUsername: "gaspargarcia_"
  },
  becca__z: {
    name: "Becca Z.",
    picture: becca__z,
    xUsername: "becca__z"
  },
  gregsoltis: {
    name: "Greg Soltis",
    picture: gregsoltis,
    xUsername: "gsoltis"
  },
  nathanhammond: {
    name: "Nathan Hammond",
    picture: nathanhammond
  },
  tomknickman: {
    name: "Tom Knickman",
    picture: tomknickman,
    xUsername: "tknickman"
  },
  mehulkar: {
    name: "Mehul Kar",
    picture: mehulkar,
    xUsername: "mehulkar"
  },
  mattpocock: {
    name: "Matt Pocock",
    picture: mattpocock,
    xUsername: "mattpocockuk"
  },
  tobiaskoppers: {
    name: "Tobias Koppers",
    picture: tobiaskoppers,
    xUsername: "wSokra"
  },
  alexkirsz: {
    name: "Alex Kirszenberg",
    picture: alexkirsz,
    xUsername: "alexkirsz"
  },
  anthonyshew: {
    name: "Anthony Shew",
    picture: anthonyshew,
    xUsername: "anthonysheww"
  },
  nicholasyang: {
    name: "Nicholas Yang",
    picture: nicholasyang,
    xUsername: "nicholaslyang"
  },
  chrisolszewski: {
    name: "Chris Olszewski",
    picture: chrisolszewski
  },
  alexanderlyon: {
    name: "Alexander Lyon",
    picture: alexanderlyon,
    xUsername: "_arlyon"
  },
  dimitrimitropoulos: {
    name: "Dimitri Mitropoulos",
    picture: dimitrimitropoulos
  }
} as const;

export type Author = keyof typeof TURBO_TEAM;
export type AuthorDetails = (typeof TURBO_TEAM)[Author];
