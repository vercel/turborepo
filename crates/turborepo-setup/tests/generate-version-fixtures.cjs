// Development-only reference generator; Rust tests consume the checked-in TSV.
// From the repository root: node crates/turborepo-setup/tests/generate-version-fixtures.cjs
// If this worktree has no node_modules, supply NODE_PATH to an installed one.
const fs = require('node:fs')
const path = require('node:path')
const semver = require('semver')
const referenceVersion = require('semver/package.json').version
if (referenceVersion !== '7.5.2') throw new Error('Use npm semver 7.5.2 for these fixtures')
const versions = [
  '0.0.0-0', '0.0.0', '0.0.1-alpha', '0.0.1', '0.0.2', '0.1.0-alpha',
  '0.1.0', '0.1.1', '0.2.0', '0.9.9', '1.0.0-0', '1.0.0-alpha', '1.0.0',
  '1.0.1', '1.1.0', '1.2.0-0', '1.2.0-alpha', '1.2.0', '1.2.1',
  '1.2.3-0', '1.2.3-alpha', '1.2.3-alpha.1', '1.2.3-alpha.2', '1.2.3-beta',
  '1.2.3', '1.2.3+one', '1.2.3+two', '1.2.4-0', '1.2.4-alpha', '1.2.4',
  '1.3.0-0', '1.3.0-alpha', '1.3.0', '1.9.9', '2.0.0-0', '2.0.0-alpha',
  '2.0.0', '2.0.1', '2.1.0', '2.3.4-rc.1', '2.3.4', '2.3.5', '2.4.0-alpha',
  '2.4.0', '2.9.9', '3.0.0-alpha', '3.0.0', '4.0.0',
  '9007199254740991.0.0', '1.9007199254740991.0', '0.0.9007199254740991',
  '9007199254740992.0.0', '1.9007199254740992.0', '1.2.9007199254740992',
  ...['-', '+'].flatMap(s => [250, 251].map(n => `1.2.3${s}${'b'.repeat(n)}`)),
  ...[248, 249].map(n => `1.2.3-b+${'b'.repeat(n)}`),
]
const requests = [
  '1.2.3', 'v1.2.3', '=1.2.3', '= v1.2.3', '1.2.3+one', 'v1.2.3-alpha.1+build.01',
  '1', '1.2', '1.x', '1.X.X', '1.2.*', '*', 'x', 'X.x.*',
  '>1', '>1.2', '>=1', '>=1.2', '<1', '<1.2', '<=1', '<=1.2',
  '>1.2.3', '>=1.2.3', '<1.2.3', '<=1.2.3', '> *', '<x', '>=*', '<=*',
  '^1', '^1.2', '^1.2.3', '^0', '^0.0', '^0.0.0', '^0.0.1', '^0.1', '^0.1.0',
  '^0.0.x', '^0.x', '^*', '~1', '~1.2', '~1.2.3', '~0.0.1', '~> 1.2.3', '~*',
  '~ > 1.2.3', '^=1.2.3', '~=1.2.3', '~>=1.2.3',
  '^= 1.2.3', '^ = 1.2.3', '~= 1.2.3', '~>= 1.2.3',
  '~ > 1.2.3 junk', '^=1.2.3 || junk', '~=1.2.3junk', '~>=1.2.3.4', '^= >=1.2.3', '~> = 1.2.3',
  '^1.2.3-alpha.1', '~1.2.3-alpha.1', '^0.0.1-alpha',
  '>=1.2.3-alpha <2', '>=1.2.3 <2.0.0-alpha', '>=1.2.3-alpha <1.2.3-beta',
  '>1.2.3-alpha.1 <=1.2.3-alpha.2', '>=1.2.3+one <=1.2.3+two',
  '1.2.3 - 2.3.4', '1.2 - 2.3', '1 - 2', '* - 2', '1 - *', '* - *',
  '1.2.3-alpha - 2.3.4-rc.1', 'v1.2.3+one - v2.3.4+two',
  '>2 <1', '>1.2.3 <=1.2.3', '>=1.2.3 <1.2.3', '1.2.3 2.3.4',
  '1.x 2.x', '^1.2.3 ~2.3.4', '2 - 1', '>=1.2.3 <1.2.4-0 >1.2.4',
  '>2 <1 || 1.2.3', '>=1 <2 || >=3 <4', '* >=1.2.3 <2',
  '>=1.2.3-alpha <1.2.3 || >=2 <3', '<* || >=1.2.3-alpha <2',
  '  >= 1.2.3   < 2  ', '9007199254740991.0.0', '=1.9007199254740991.0',
  '^9007199254740991.0.0', '^0.0.9007199254740991', '>9007199254740991',
  '>=1.2.3 garbage', '>=1.2.3 || garbage', '>=1.2.3 <nope', '1.2.3 - nope',
  '1.2.3, <2', '=>1.2.3', '1.2.3.4', '1.2.3-01', '1.2.3+', '1.2.3+foo..bar',
  '01.2.3', '1.02', 'latest', 'node', 'lts/*', 'https://example.com',
]
const lines = requests.map(request => {
  let result
  try {
    const range = new semver.Range(request)
    result = versions.map(version => range.test(version) ? '1' : '0').join('')
  } catch {
    result = 'invalid'
  }
  return `${request}\t${result}`
})
fs.writeFileSync(path.join(__dirname, 'version-requests.tsv'), [
  `# npm semver ${referenceVersion}; default strict options`,
  `# releases\t${versions.join(' ')}`, ...lines, '',
].join('\n'))
