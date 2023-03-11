import chalk from "chalk";
import ora from "ora";
import gradient from "gradient-string";

const BLUE = "#0099F7";
const RED = "#F11712";

export const turboGradient = gradient(BLUE, RED);
export const turboBlue = chalk.hex(BLUE);
export const turboRed = chalk.hex(RED);

export const turboLoader = (text: string) =>
  ora({
    text,
    spinner: {
      frames: ["   ", turboBlue(">  "), turboBlue(">> "), turboBlue(">>>")],
    },
  });

export const info = (...args: any[]) => {
  console.log(turboBlue.bold(">>>"), ...args);
};

export const error = (...args: any[]) => {
  console.error(turboRed.bold(">>>"), ...args);
};
