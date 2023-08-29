import chalk from "chalk";
import ora from "ora";
import gradient from "gradient-string";

const BLUE = "#0099F7";
const RED = "#F11712";
const YELLOW = "#FFFF00";

export const turboGradient = gradient(BLUE, RED);
export const turboBlue = chalk.hex(BLUE);
export const turboRed = chalk.hex(RED);
export const yellow = chalk.hex(YELLOW);

export const turboLoader = (text: string) =>
  ora({
    text,
    spinner: {
      frames: ["   ", turboBlue(">  "), turboBlue(">> "), turboBlue(">>>")],
    },
  });

export const info = (...args: Array<unknown>) => {
  // eslint-disable-next-line no-console
  console.log(turboBlue.bold(">>>"), ...args);
};

export const error = (...args: Array<unknown>) => {
  // eslint-disable-next-line no-console
  console.error(turboRed.bold(">>>"), ...args);
};

export const warn = (...args: Array<unknown>) => {
  // eslint-disable-next-line no-console
  console.error(yellow.bold(">>>"), ...args);
};

export const bold = (...args: Array<string>) => {
  // eslint-disable-next-line no-console
  console.log(chalk.bold(...args));
};

export const dimmed = (...args: Array<string>) => {
  // eslint-disable-next-line no-console
  console.log(chalk.dim(...args));
};

export const item = (...args: Array<unknown>) => {
  // eslint-disable-next-line no-console
  console.log(turboBlue.bold("  •"), ...args);
};

export const log = (...args: Array<unknown>) => {
  // eslint-disable-next-line no-console
  console.log(...args);
};
