//! Tests of the environment facts ([`super::env`]).

use turborepo_trace_core::{
    Origin,
    env::{EnvAlt, EnvBase, EnvFact, EnvName, EnvPart, EnvTerm, EnvValue},
};

use crate::{
    extract::{ScanOptions, scan_named},
    lexer::Dialect,
};

fn facts(name: &str, src: &str) -> Vec<EnvFact> {
    let dialect = Dialect::from_path(std::path::Path::new(name));
    scan_named(
        src.as_bytes(),
        dialect,
        ScanOptions {
            env_facts: true,
            ..ScanOptions::default()
        },
        Some(name),
    )
    .facts
}

fn sets(fs: &[EnvFact]) -> Vec<(String, u32, EnvValue)> {
    fs.iter()
        .filter_map(|f| match f {
            EnvFact::Set {
                name, line, value, ..
            } => Some((name.clone(), *line, value.clone())),
            _ => None,
        })
        .collect()
}

fn uses(fs: &[EnvFact]) -> Vec<(u32, Origin, EnvValue)> {
    fs.iter()
        .filter_map(|f| match f {
            EnvFact::Use {
                line,
                origin,
                value,
                ..
            } => Some((*line, Origin::ALL[*origin as usize], value.clone())),
            _ => None,
        })
        .collect()
}

fn helper<'a>(fs: &'a [EnvFact], name: &str) -> Option<&'a EnvValue> {
    fs.iter().find_map(|f| match f {
        EnvFact::Helper { export, value } if export == name => Some(value),
        _ => None,
    })
}

fn term(base: EnvBase, parts: Vec<EnvPart>) -> EnvAlt {
    EnvAlt::Term(EnvTerm { base, parts })
}

fn lit(s: &str) -> EnvAlt {
    EnvAlt::Term(EnvTerm::str(s))
}

fn var(name: &str) -> EnvPart {
    EnvPart::Var(EnvName::Name(name.to_owned()))
}

/// `readFile(envOr(NAME, DEFAULT))` with a helper imported from a workspace
/// package.
#[test]
fn use_through_an_imported_helper() {
    let fs = facts(
        "client.ts",
        "import { envOr } from '@acme/settings';\nimport { readFile } from \
         'node:fs/promises';\nconst CERT = '/etc/ssl/acme/client.pem';\nexport async function \
         clientCert() {\n  return readFile(envOr('ACME_CLIENT_CERT', CERT), 'utf8');\n}\n",
    );
    let us = uses(&fs);
    assert_eq!(us.len(), 1, "{fs:?}");
    let (line, origin, value) = &us[0];
    assert_eq!((*line, *origin), (5, Origin::Call));
    assert_eq!(value.alts, [term(EnvBase::Str, vec![EnvPart::Call(0)])]);
    let c = &value.calls[0];
    assert_eq!(
        (c.module.as_str(), c.export.as_str()),
        ("@acme/settings", "envOr")
    );
    assert_eq!(c.args[0].value, [lit("ACME_CLIENT_CERT")]);
    assert_eq!(c.args[1].value, [lit("/etc/ssl/acme/client.pem")]);
}

/// The helper call bound to a constant first, and a helper with a callback
/// fallback.
#[test]
fn use_through_a_const_and_a_callback() {
    let fs = facts(
        "client.ts",
        "import fs from 'node:fs';\nimport { required, envOrElse } from '@acme/settings';\nconst \
         keyFile = required('SIGNING_KEY_FILE');\nexport const a = () => fs.readFileSync(keyFile, \
         'utf8');\nexport const b = () => fs.readFileSync(envOrElse('B_FILE', () => '/etc/b'), \
         'utf8');\n",
    );
    let us = uses(&fs);
    assert_eq!(us.len(), 2, "{fs:?}");
    assert_eq!(us[0].2.calls[0].export, "required");
    assert_eq!(us[0].2.calls[0].args[0].value, [lit("SIGNING_KEY_FILE")]);
    let cb = &us[1].2.calls[0];
    assert_eq!(cb.export, "envOrElse");
    assert_eq!(cb.args[1].value, []);
    assert_eq!(cb.args[1].returns, Some(vec![lit("/etc/b")]));
}

/// `process.env.X` through a const and a guard.
#[test]
fn use_of_a_variable_through_a_const_and_a_guard() {
    let fs = facts(
        "client.ts",
        "import { readFileSync } from 'node:fs';\nconst pricesPath = \
         process.env.PRICES_FIXTURE;\nexport const prices = pricesPath\n  ? \
         JSON.parse(readFileSync(pricesPath, 'utf8'))\n  : [];\n",
    );
    let us = uses(&fs);
    assert_eq!(us.len(), 1, "{fs:?}");
    assert_eq!(us[0].1, Origin::Env);
    assert_eq!(
        us[0].2.alts,
        [term(EnvBase::Str, vec![var("PRICES_FIXTURE")])]
    );
}

/// A variable as the head of a path, a fallback, and a derived value.
#[test]
fn use_templates() {
    let fs = facts(
        "tool.mjs",
        "import fs from 'node:fs';\nimport path from \
         'node:path';\nfs.readFileSync(path.join(process.env.DATA_DIR, 'seed', \
         'a.json'));\nfs.readFileSync(process.env.CONFIG_FILE ?? path.join(import.meta.dirname, \
         'default.json'));\nfs.readFileSync(process.env.LIST.split(',')[0]);\nrequire(process.env.\
         PLUGIN_MODULE);\n",
    );
    let us = uses(&fs);
    assert_eq!(us.len(), 4, "{fs:?}");
    assert_eq!(
        us[0].2.alts,
        [term(
            EnvBase::Str,
            vec![var("DATA_DIR"), EnvPart::Lit("/seed/a.json".into())]
        )]
    );
    // The fallback is an ordinary reference of the site; the fact has the variable.
    assert_eq!(us[1].2.alts, [term(EnvBase::Str, vec![var("CONFIG_FILE")])]);
    // A value derived from a variable is not the variable.
    assert!(matches!(us[2].2.alts.as_slice(), [EnvAlt::Unknown(_)]));
    assert_eq!(
        us[3].2.alts,
        [term(EnvBase::Str, vec![var("PLUGIN_MODULE")])]
    );
}

/// Assignments: `process.env.X = v`, restore from a saved value, a test
/// server's environment object, `stubEnv`, a spawn `env`, a shell command line.
#[test]
fn assignments() {
    let fs = facts(
        "setup.test.ts",
        "import path from 'node:path';\nimport { vi } from 'vitest';\nimport { spawn } from \
         'node:child_process';\nconst original = process.env.PRICES_FIXTURE;\nbeforeEach(() => \
         {\n  process.env.PRICES_FIXTURE = '/tmp/prices-fixture.json';\n  \
         process.env['FIXTURE_FILE'] = path.join(__dirname, 'fixtures', 'a.json');\n  \
         vi.stubEnv('STUBBED_FILE', path.resolve(__dirname, \
         '../data/b.json'));\n});\nafterEach(() => { process.env.PRICES_FIXTURE = original; \
         });\nclass Harness {\n  private readonly catalog = path.join(__dirname, \
         'catalog.json');\n  async childEnv() {\n    return { HARNESS_CATALOG: this.catalog, \
         HARNESS_TOKEN: 'fake' };\n  }\n}\nspawn('node', ['x.js'], { env: { ...process.env, \
         CHILD_CONFIG: computeIt() } });\nexecSync('SHELL_FILE=./conf/c.json node \
         run.js');\nprocess.env.NOT_A_PATH = undefined;\n",
    );
    let got: Vec<(String, u32, Vec<EnvAlt>)> = sets(&fs)
        .into_iter()
        .map(|(n, l, v)| (n, l, v.alts))
        .collect();
    let find = |name: &str| -> Vec<&Vec<EnvAlt>> {
        got.iter().filter(|g| g.0 == name).map(|g| &g.2).collect()
    };
    assert_eq!(
        find("PRICES_FIXTURE"),
        [
            &vec![lit("/tmp/prices-fixture.json")],
            // Restoring the saved value: the variable itself.
            &vec![term(EnvBase::Str, vec![var("PRICES_FIXTURE")])],
        ],
        "{got:?}"
    );
    assert_eq!(
        find("FIXTURE_FILE"),
        [&vec![term(
            EnvBase::FileDir,
            vec![EnvPart::Lit("fixtures/a.json".into())]
        )]]
    );
    assert_eq!(
        find("STUBBED_FILE"),
        [&vec![term(
            EnvBase::FileDir,
            vec![EnvPart::Lit("../data/b.json".into())]
        )]]
    );
    assert_eq!(
        find("HARNESS_CATALOG"),
        [&vec![term(
            EnvBase::FileDir,
            vec![EnvPart::Lit("catalog.json".into())]
        )]]
    );
    assert_eq!(find("HARNESS_TOKEN"), [&vec![lit("fake")]]);
    assert!(matches!(
        find("CHILD_CONFIG").as_slice(),
        [alts] if matches!(alts.as_slice(), [EnvAlt::Unknown(_)])
    ));
    assert_eq!(find("SHELL_FILE"), [&vec![lit("./conf/c.json")]]);
    // Never a path: not recorded.
    assert!(find("NOT_A_PATH").is_empty());
    // The line of the first assignment.
    assert_eq!(got.iter().find(|g| g.0 == "PRICES_FIXTURE").unwrap().1, 6);
}

/// Assertions and test names are not assignments; a temporary path is outside
/// the repository.
#[test]
fn not_assignments() {
    let fs = facts(
        "a.test.ts",
        "import os from 'node:os';\nimport path from \
         'node:path';\nexpect(spy).toHaveBeenCalledWith('TOKEN_FILE', \
         expect.anything());\nit('TOKEN_FILE', () => {});\nconsole.log('TOKEN_FILE', \
         value);\nprocess.env.TOKEN_FILE = path.join(os.tmpdir(), 'token');\ndelete \
         process.env.TOKEN_FILE;\nconst x = process.env.TOKEN_FILE === 'TOKEN_FILE';\n",
    );
    assert_eq!(sets(&fs), [], "{fs:?}");
}

/// The helper family: a variable named by a parameter, with a fallback
/// parameter, a fallback callback, a rest parameter, and a function that
/// delegates.
#[test]
fn helpers() {
    let fs = facts(
        "index.ts",
        "export function firstSet(...keys: string[]): string {\n  for (const key of keys) {\n    \
         const found = process.env[key];\n    if (found) {\n      return found;\n    }\n  }\n  \
         if (process.env.ENV_PLACEHOLDERS === '1') {\n    return keys[0] ?? 'PLACEHOLDER';\n  \
         }\n  throw new Error('none of ' + keys.join(' / ') + ' is set');\n}\nexport function \
         required(key: string): string {\n  return firstSet(key);\n}\nexport function envOr(key: \
         string, otherwise: string): string {\n  const found = process.env[key];\n  if (!found) \
         {\n    note(key);\n    return otherwise;\n  }\n  return found;\n}\nexport function \
         envOrElse(key: string, otherwise: () => string): string {\n  const found = \
         process.env[key];\n  if (!found) {\n    const computed = otherwise();\n    note(key);\n    \
         return computed;\n  }\n  return found;\n}\nexport function inTests(): boolean {\n  \
         return process.env.CI_STAGE === 'unit';\n}\nexport function upper(key: string): string \
         {\n  return (process.env[key] ?? '').toUpperCase();\n}\nfunction note(key: string) {}\n",
    );
    let arg_var = |i: u32| term(EnvBase::Str, vec![EnvPart::Var(EnvName::Arg(i))]);
    let arg = |i: u32| term(EnvBase::Str, vec![EnvPart::Arg(i)]);
    assert_eq!(
        helper(&fs, "envOr").map(|v| v.alts.clone()),
        Some(vec![arg(1), arg_var(0)]),
        "{fs:?}"
    );
    assert_eq!(
        helper(&fs, "envOrElse").map(|v| v.alts.clone()),
        Some(vec![
            term(EnvBase::Str, vec![EnvPart::ArgCall(1)]),
            arg_var(0)
        ])
    );
    let any = helper(&fs, "required").expect("required").alts.clone();
    assert!(any.contains(&arg_var(0)), "{any:?}");
    assert!(any.contains(&arg(0)), "{any:?}");
    assert!(any.contains(&lit("PLACEHOLDER")), "{any:?}");
    let or = helper(&fs, "firstSet").expect("firstSet").alts.clone();
    for i in 0..4 {
        assert!(or.contains(&arg_var(i)), "{or:?}");
    }
    // Not a string, and a derived value: no helper.
    assert_eq!(helper(&fs, "inTests"), None);
    assert_eq!(helper(&fs, "upper"), None);
}

/// Files without references still give their assignments; files without
/// candidates are not parsed.
#[test]
fn files_without_references() {
    let fs = facts(
        "env.js",
        "export const testEnv = { DATA_FILE: '/srv/data.json' };\n",
    );
    assert_eq!(
        sets(&fs)
            .into_iter()
            .map(|(n, _, v)| (n, v.alts))
            .collect::<Vec<_>>(),
        [("DATA_FILE".to_owned(), vec![lit("/srv/data.json")])]
    );
    // Constants, typed declarations, case labels, import lists, string values and
    // comments are not assignments of a variable.
    assert!(!super::env::may_have_facts(
        b"import { AAA, BBB } from './c';\nconst MAX_SIZE = 3;\nconst X_Y: number = 1;\n\
          export const a = MAX_SIZE + 1;\nfoo(a == B_C, MAX_SIZE);\n\
          switch (a) { case FOO_BAR: break; }\nconst m = { method: 'POST', other: [AAA] };\n\
          // NOTE: see DATA_FILE: docs\nconst t = a ? AAA : BBB;\n"
    ));
    for src in [
        "x = { DATA_FILE: 1 }",
        "process.env.DATA_FILE = 'a'",
        "stub('DATA_FILE', v)",
        "env['DATA_FILE'] = v",
        "run(`DATA_FILE=${f} cmd`)",
        "process.env[name]",
        "process.env.lower ??= v",
        "x = {\n  // why\n  DATA_FILE: v,\n}",
        "x = { a: 1, /* c */ 'DATA_FILE': v }",
        "spawn(c, { env: { ...process.env, DATA_FILE } })",
        "set(v,\n  'DATA_FILE',\n)",
        "cfg.DATA_FILE ||= v",
    ] {
        assert!(super::env::may_have_facts(src.as_bytes()), "{src}");
    }
}

/// Facts are off without the scan option, and for declaration files.
#[test]
fn facts_are_opt_in() {
    let ex = scan_named(
        b"process.env.DATA_FILE = '/srv/x';\n",
        Dialect::JS,
        ScanOptions::default(),
        Some("a.js"),
    );
    assert!(ex.facts.is_empty());
}
