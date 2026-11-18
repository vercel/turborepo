const { spawn } = require('child_process');

function launch({ label, tool }) {
  const argv = process.argv.slice(2).filter((arg) => !arg.startsWith('--shard='));
  const env = Object.assign({}, process.env, { CI_REPORT_LABEL: label });
  if (tool === 'vitest') {
    env.NODE_OPTIONS = `--require ${require.resolve('ci-reporter/preload.js')}`;
  }
  spawn('pnpm', ['exec', tool, ...argv], { env, stdio: 'inherit' }).on('exit', (status) =>
    process.exit(status === null ? 1 : status),
  );
}

module.exports = { launch };
