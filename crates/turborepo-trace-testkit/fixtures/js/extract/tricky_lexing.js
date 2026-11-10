// Regex vs division, templates and ASI-sensitive spots, with decoys everywhere.
const a = b / c / d; require('./after-division');
const r1 = /'/g; require('./after-regex-with-quote');
const r2 = /[/"]/; require('./after-regex-with-class');
const r3 = x.replace(/`/g, ''); require('./after-regex-with-backtick');
if (ok) /'/.test(s); require('./after-if-regex');
function f() { return /"/.test(s) }
f() / 2; require('./after-call-division');
const t1 = `outer ${`inner ${require('./nested-template-sub')}`} require('./not-a-ref')`;
const t2 = `${ { key: '}' }.key }${ '`' }`; require('./after-object-in-template');
const t3 = tag`import x from './not-a-ref'`;
const obj = { a: 1 } / 2; require('./after-object-division');
{ } /'/.test(s); require('./after-block-regex');
x = y++ / 2; require('./after-postfix');
const keywordsAsProps = o.return / o.typeof / 2; require('./after-keyword-props');
switch (k) { case /'/.source: break; default: require('./in-switch'); }
label: { require('./in-label'); }
const big = 1_000n / 0x1Fn; require('./after-numbers');
const unicode = café / ü / 2; require('./after-unicode');
const priv = class { #x = 1; m() { return this.#x / 2 } }; require('./after-private');
const html = '<div>require("./not-a-ref")</div>';
