const fs = require('node:fs');
const path = require('node:path');
const { createRequire } = require('node:module');

function installedLauncher(binary, jestPath) {
  fs.accessSync(binary, fs.constants.X_OK);
  const target = path.join(path.dirname(jestPath), 'bin', 'jest.js');
  if (fs.realpathSync(binary) === fs.realpathSync(target)) return true;
  const directory = path.dirname(binary);
  if (fs.existsSync(path.join(directory, 'node')) || fs.existsSync(path.join(directory, 'node.exe'))) return false;
  const stat = fs.statSync(binary);
  if (!stat.isFile() || stat.size > 16 * 1024) return false;
  const source = fs.readFileSync(binary, 'utf8');
  if (!source.startsWith('#!/bin/sh\n')) return false;
  const nodePath = source.match(/^\s*export NODE_PATH="([^"$`]+)"$/m)?.[1];
  if (!nodePath || nodePath.includes('\\')) return false;
  const packageDirectory = path.dirname(fs.realpathSync(jestPath));
  const paths = nodePath.split(path.delimiter);
  if (paths.length !== 3 || paths[0] !== path.join(packageDirectory, 'node_modules') || paths[1] !== path.dirname(packageDirectory) || paths[2] !== path.join(path.dirname(path.dirname(path.dirname(packageDirectory))), 'node_modules')) return false;
  const normalize = text => text.split('\n').map(line => line.trim()).filter(line => line && !line.startsWith('#')).join('\n');
  const actual = normalize(source.replaceAll(nodePath, '<node_path>'));
  const known = String.raw`
link="$0"
hops=0
while [ -L "$link" ] && [ "$hops" -lt 40 ]; do
  hops=$((hops+1))
  target=$(readlink "$link")
  case "$target" in
    /*) link="$target" ;;
    *)  link="$(dirname "$link")/$target" ;;
  esac
done
basedir=$(dirname "$(echo "$link" | sed -e 's,\\,/,g')")
basedir_win="$basedir"
exe=""
msys=""
case \`uname -a\` in
  *CYGWIN*|*MINGW*|*MSYS*)
    if command -v cygpath > /dev/null 2>&1; then
      basedir_win=\`cygpath -w "$basedir"\`
    fi
    exe=".exe"
    msys="true"
  ;;
  *WSL2*)
    if command -v wslpath > /dev/null 2>&1; then
      basedir_win="$(wslpath -w "$basedir" 2> /dev/null)"
      if [ $? -ne 0 ] || [ -z "$basedir_win" ]; then
        basedir_win="$basedir"
      else
        exe=".exe"
      fi
    fi
  ;;
esac
if [ -z "$NODE_PATH" ]; then
  export NODE_PATH="<node_path>"
else
  export NODE_PATH="<node_path>:$NODE_PATH"
fi
if [ -n "$exe" ] && [ -x "$basedir/node.exe" ]; then
  exec "$basedir/node.exe"  "$basedir_win/../jest/bin/jest.js" "$@"
elif [ -x "$basedir/node" ]; then
  exec "$basedir/node"  "$basedir/../jest/bin/jest.js" "$@"
elif command -v node >/dev/null 2>&1; then
  exec node  "$basedir/../jest/bin/jest.js" "$@"
elif [ -n "$exe" ] && command -v node.exe >/dev/null 2>&1; then
  exec node.exe  "$basedir_win/../jest/bin/jest.js" "$@"
else
  exec node  "$basedir/../jest/bin/jest.js" "$@"
fi`.replaceAll('\\`', '`');
  return actual === normalize(known) && fs.realpathSync(path.join(directory, '../jest/bin/jest.js')) === fs.realpathSync(target);
}

async function main() {
  const input = JSON.parse(process.argv[2]);
  const local = createRequire(path.join(process.cwd(), 'package.json'));
  const jestPath = local.resolve('jest/package.json');
  const jestRequire = createRequire(jestPath);
  const version = jestRequire('./package.json').version;
  let binary;
  for (let directory = process.cwd();; directory = path.dirname(directory)) {
    const candidate = path.join(directory, 'node_modules', '.bin', 'jest');
    if (fs.existsSync(candidate)) { binary = candidate; break; }
    if (directory === path.dirname(directory)) break;
  }
  if (input.executable) binary = input.executable;
  if (!binary || !installedLauncher(binary, jestPath)) throw new Error('Jest command does not match the installed runner');
  if (!version.startsWith('30.')) throw new Error('Jest 30 is required');
  const { runCLI, SearchSource } = jestRequire('@jest/core');
  const { readConfigs } = jestRequire('jest-config');
  const argv = { $0: 'jest', _: [], runInBand: true };
  if (input.mode === 'run') {
    const { results } = await runCLI({ ...argv, _: input.tests, runTestsByPath: true }, [process.cwd()]);
    fs.writeFileSync(input.result, JSON.stringify({
      success: results.success,
      files: results.testResults.map(file => ({
        path: file.testFilePath,
        passed: file.numPassingTests,
        failed: file.numFailingTests,
        pending: file.numPendingTests + file.numTodoTests,
        error: Boolean(file.testExecError)
      }))
    }));
    process.exitCode = results.success ? 0 : 1;
    return;
  }
  const root = fs.realpathSync(input.root);
  for (let directory = fs.realpathSync(process.cwd());; directory = path.dirname(directory)) {
    for (const extension of ['js', 'cjs', 'mjs', 'ts', 'cts', 'mts', 'json']) {
      if (fs.existsSync(path.join(directory, `jest.config.${extension}`))) throw new Error('external Jest config');
    }
    if (directory === root) break;
    if (directory === path.dirname(directory)) throw new Error('outside repository');
  }
  const packageJson = JSON.parse(fs.readFileSync('package.json', 'utf8'));
  const allowed = new Set(['transform', 'testEnvironment', 'testMatch', 'testRegex', 'moduleFileExtensions', 'testPathIgnorePatterns', 'verbose', 'clearMocks', 'resetMocks', 'restoreMocks']);
  if (!packageJson.jest || typeof packageJson.jest !== 'object' || Array.isArray(packageJson.jest) || Object.keys(packageJson.jest).some(key => !allowed.has(key))) throw new Error('unsupported Jest option');
  if (packageJson.scripts?.[`pre${input.task}`] || packageJson.scripts?.[`post${input.task}`]) {
    throw new Error('package lifecycle scripts');
  }
  const { configs, globalConfig } = await readConfigs(argv, [process.cwd()]);
  if (configs.length !== 1) throw new Error('multiple projects');
  const config = configs[0];
  const arrays = ['transform', 'setupFiles', 'setupFilesAfterEnv', 'snapshotSerializers', 'moduleNameMapper', 'modulePaths'];
  if (arrays.some(key => config[key]?.length)) throw new Error('custom transforms, setup or resolution');
  if (config.rootDir !== process.cwd() || config.roots.length !== 1 || config.roots[0] !== process.cwd()) throw new Error('custom roots');
  if (config.resolver || config.dependencyExtractor || config.globalSetup || config.globalTeardown || config.preset || config.automock || config.injectGlobals === false) throw new Error('custom runtime');
  if (Object.keys(config.testEnvironmentOptions).length || !config.testEnvironment.includes('jest-environment-node') || !config.runner.includes('jest-runner') || !config.testRunner.includes('jest-circus')) throw new Error('custom runner or environment');
  if (config.moduleDirectories.join(',') !== 'node_modules' || config.haste.hasteImplModulePath || config.haste.hasteMapModulePath) throw new Error('custom module discovery');
  if (globalConfig.updateSnapshot === 'all' || globalConfig.passWithNoTests || globalConfig.collectCoverage || globalConfig.watch || globalConfig.watchAll || globalConfig.onlyChanged || globalConfig.findRelatedTests || globalConfig.shard || globalConfig.projects.length || globalConfig.filter || globalConfig.reporters) throw new Error('custom execution');
  const Runtime = jestRequire('jest-runtime').default;
  const haste = await Runtime.createHasteMap(config, { maxWorkers: 1, watchman: false });
  const { hasteFS, moduleMap } = await haste.build();
  const context = { config, hasteFS, moduleMap, resolver: Runtime.createResolver(config, moduleMap) };
  const { tests } = await new SearchSource(context).getTestPaths(globalConfig, config);
  if (hasteFS.getAllFiles().some(file => file.split(path.sep).includes('__mocks__') || file.endsWith('.snap'))) throw new Error('mocks or snapshots');
  fs.writeFileSync(input.result, JSON.stringify({ tests: tests.map(test => test.path).sort(), config, version, node: process.version, platform: process.platform, arch: process.arch, extensions: config.moduleFileExtensions }));
}
main().catch(error => { process.stderr.write(`${error.message}\n`); process.exitCode = 1; });
