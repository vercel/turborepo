const { spawn } = require('child_process');

function runTestRunnerWithDDTrace({ runner, defaultSessionName }) {
  const args = process.argv.slice(2).filter((a) => !a.startsWith('--type='));
  const env = { ...process.env, DD_SESSION: defaultSessionName };
  if (runner === 'vitest') {
    env.NODE_OPTIONS = `--import ${require.resolve('dd-trace/register.js')}`;
  }
  const child = spawn('pnpm', ['exec', runner, ...args], { stdio: 'inherit', env });
  child.on('exit', (code) => process.exit(code ?? 1));
}

module.exports = { runTestRunnerWithDDTrace };
