import {
  reset,
  bold as pcBold,
  underline as pcUnderline,
  gray,
  dim,
} from "picocolors";
import ora from "ora";
import gradient from "gradient-string";

const BLUE = "#0099F7";
const RED = "#F11712";
const YELLOW = "#FFFF00";

const hex = (color: string): ((text: string) => string) => {
  const ansiColor = hexToAnsi256(color);
  return (text: string) => `\x1b[38;5;${ansiColor}m${text}${reset("")}`;
};

export const turboGradient = gradient(BLUE, RED);
export const turboBlue = hex(BLUE);
export const turboRed = hex(RED);
export const yellow = hex(YELLOW);

export const turboLoader = (text: string) =>
  ora({
    text,
    spinner: {
      frames: ["   ", turboBlue(">  "), turboBlue(">> "), turboBlue(">>>")],
    },
  });

export const info = (...args: Array<unknown>) => {
  log(turboBlue(pcBold(">>>")), args.join(" "));
};

export const bold = (...args: Array<string>) => {
  log(pcBold(args.join(" ")));
};

export const underline = (...args: Array<string>) => {
  log(pcUnderline(args.join(" ")));
};

export const dimmed = (...args: Array<string>) => {
  log(dim(args.join(" ")));
};

export const grey = (...args: Array<string>) => {
  log(gray(args.join(" ")));
};

export const item = (...args: Array<unknown>) => {
  log(turboBlue(pcBold("  •")), args.join(" "));
};

export const log = (...args: Array<unknown>) => {
  // eslint-disable-next-line no-console -- logger
  console.log(...args);
};

export const warn = (...args: Array<unknown>) => {
  // eslint-disable-next-line no-console -- warn logger
  console.error(yellow(pcBold(">>>")), args.join(" "));
};

export const error = (...args: Array<unknown>) => {
  // eslint-disable-next-line no-console -- error logger
  console.error(turboRed(pcBold(">>>")), args.join(" "));
};

function hexToAnsi256(sHex: string): number {
  const rgb = parseInt(sHex.slice(1), 16);
  const r = Math.floor(rgb / (256 * 256)) % 256;
  const g = Math.floor(rgb / 256) % 256;
  const b = rgb % 256;

  const ansi =
    16 +
    36 * Math.round((r / 255) * 5) +
    6 * Math.round((g / 255) * 5) +
    Math.round((b / 255) * 5);
  return ansi;
}
