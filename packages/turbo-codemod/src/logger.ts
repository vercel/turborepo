import chalk from "chalk";

function skip(...args: any[]) {
  console.log(chalk.yellow.inverse(` SKIP `), ...args);
}
function error(...args: any[]) {
  console.log(chalk.red.inverse(` ERROR `), ...args);
}
function ok(...args: any[]) {
  console.log(chalk.green.inverse(` OK `), ...args);
}

export { skip, error, ok };
