//! Tests of guards ([`super::guard`]) through the extractor: what a test around
//! a load bounds, and the shapes that only look like one and must leave the
//! load open.

use super::tests::show;

/// A load of an escaping function's parameter: open.
const OPEN: &str = "require dir:<unknown> !computed_specifier/param";

#[test]
fn a_prefix_test_that_exits_bounds_the_load() {
    // throw, return, continue and break, bare or as the end of a block.
    for exit in [
        "throw new Error('outside');",
        "return null;",
        "{ console.warn(name); throw new Error('outside'); }",
    ] {
        assert_eq!(
            show(&format!(
                "export function load(name: string) {{\n  if (!name.startsWith('./handlers/')) \
                 {exit}\n  return require(name);\n}}\n"
            )),
            "require dir:./handlers/* !partial_specifier/unknown",
            "{exit}"
        );
    }
    for exit in ["continue;", "break;"] {
        assert_eq!(
            show(&format!(
                "export function load(names: string[]) {{\n  for (const name of names) {{\n    if \
                 (!name.startsWith('./handlers/')) {exit}\n    require(name);\n  }}\n}}\n"
            )),
            "require dir:./handlers/* !partial_specifier/unknown",
            "{exit}"
        );
    }
}

#[test]
fn a_prefix_test_around_the_load_bounds_it() {
    let bounded = "require dir:./handlers/* !partial_specifier/unknown";
    // The consequent of an `if`, of a conditional expression, the right side of
    // `&&`, the alternate of a negated test, the right side of `||` of a
    // negated test.
    for body in
        [
            "if (name.startsWith('./handlers/')) { return require(name); }\n  return null;",
            "return name.startsWith('./handlers/') ? require(name) : null;",
            "return name.startsWith('./handlers/') && require(name);",
            "if (!name.startsWith('./handlers/')) { return null; } else { return require(name); }",
            "return !name.startsWith('./handlers/') || require(name);",
            "if (typeof name === 'string' && name.startsWith('./handlers/')) return require(name);",
            "if (typeof name !== 'string' || !name.startsWith('./handlers/')) throw new \
             Error('x');\n  return require(name);",
            "if (name?.startsWith('./handlers/')) return require(name);",
            "if (!name.startsWith('./handlers/')) throw new Error('x'); else { return \
             require(name); }",
        ]
    {
        assert_eq!(
            show(&format!(
                "export function load(name: any) {{\n  {body}\n}}\n"
            )),
            bounded,
            "{body}"
        );
    }
}

#[test]
fn a_test_holds_inside_a_function_made_behind_it() {
    // The shape of a framework that turns a handler path into a lazy loader.
    assert_eq!(
        show(
            "export function normalize(spec: unknown) {\n  if (typeof spec === 'string') {\n    \
             if (!spec.startsWith('./handlers/')) throw new Error('outside');\n    return [{ \
             handler: () => import(spec), modulePath: spec }];\n  }\n  return [];\n}\n"
        ),
        "dynamic_import dir:./handlers/* !partial_specifier/unknown"
    );
    // The test inside the function that loads.
    assert_eq!(
        show(
            "export function normalize(spec: string) {\n  return [{ handler: async () => {\n    \
             if (!spec.startsWith('./handlers/')) throw new Error('outside');\n    return \
             import(spec);\n  } }];\n}\n"
        ),
        "dynamic_import dir:./handlers/* !partial_specifier/unknown"
    );
}

#[test]
fn the_prefix_is_evaluated_like_any_expression() {
    // A constant, a path below the file's directory, several values.
    assert_eq!(
        show(
            "const DIR = './plugins/';\nexport function load(name: string) {\n  if \
             (!name.startsWith(DIR)) throw new Error('x');\n  return require(name);\n}\n"
        ),
        "require dir:./plugins/* !partial_specifier/unknown"
    );
    assert_eq!(
        show(
            "const base = path.join(__dirname, 'plugins');\nexport function load(file: string) \
             {\n  if (!file.startsWith(base + path.sep)) throw new Error('x');\n  return \
             require(file);\n}\n"
        ),
        "require dir:./plugins/* !partial_specifier/unknown"
    );
    assert_eq!(
        show(
            "export function load(name: string) {\n  if (!name.startsWith('./a/') && \
             !name.startsWith('./b/')) throw new Error('x');\n  return require(name);\n}\n"
        ),
        "require dir:./a/* | ./b/* !partial_specifier/unknown"
    );
    // The value flows on: a binding made from the tested one.
    assert_eq!(
        show(
            "export function load(name: string) {\n  if (!name.startsWith('./handlers/')) throw \
             new Error('x');\n  const file = name + '.js';\n  return require(file);\n}\n"
        ),
        "require dir:./handlers/*.js !partial_specifier/unknown"
    );
}

#[test]
fn membership_in_a_list_of_the_file_bounds_the_load() {
    let both = "require dir:./a | ./b";
    assert_eq!(
        show(
            "const ALLOWED = ['./a', './b'];\nexport function load(name: string) {\n  if \
             (!ALLOWED.includes(name)) throw new Error('x');\n  return require(name);\n}\n"
        ),
        both
    );
    assert_eq!(
        show(
            "const ALLOWED = new Set(['./a', './b']);\nexport function load(name: string) {\n  if \
             (ALLOWED.has(name)) return require(name);\n}\n"
        ),
        both
    );
}

#[test]
fn an_anchored_regular_expression_bounds_the_load() {
    assert_eq!(
        show(
            "export function load(name: string) {\n  if \
             (!/^\\.\\/(handlers|hooks)\\/[\\w-]+$/.test(name)) throw new Error('x');\n  return \
             require(name);\n}\n"
        ),
        "require dir:./handlers/* | ./hooks/* !partial_specifier/unknown"
    );
}

#[test]
fn shapes_that_only_look_like_a_guard_leave_the_load_open() {
    for (why, body) in [
        (
            "the test does not exit",
            "if (!name.startsWith('./handlers/')) console.warn('outside');\n  return \
             require(name);",
        ),
        (
            "the load is in the branch where the test failed",
            "if (name.startsWith('./handlers/')) return null;\n  return require(name);",
        ),
        (
            "the load is in the alternate",
            "if (name.startsWith('./handlers/')) { return null; } else { return require(name); }",
        ),
        (
            "the test comes after the load",
            "const m = require(name);\n  if (!name.startsWith('./handlers/')) throw new \
             Error('x');\n  return m;",
        ),
        (
            "one side of || says nothing",
            "if (name.startsWith('./handlers/') || other) return require(name);",
        ),
        (
            "an exit behind && with something unknown",
            "if (!name.startsWith('./handlers/') && other) throw new Error('x');\n  return \
             require(name);",
        ),
        (
            "another binding is tested",
            "if (!other.startsWith('./handlers/')) throw new Error('x');\n  return require(name);",
        ),
        (
            "the prefix is not known",
            "if (!name.startsWith(other)) throw new Error('x');\n  return require(name);",
        ),
        (
            "an empty prefix",
            "if (!name.startsWith('')) throw new Error('x');\n  return require(name);",
        ),
        (
            "startsWith from a position",
            "if (!name.startsWith('./handlers/', 2)) throw new Error('x');\n  return \
             require(name);",
        ),
        (
            "endsWith",
            "if (!name.endsWith('.js')) throw new Error('x');\n  return require(name);",
        ),
        (
            "a substring test",
            "if (!name.includes('./handlers/')) throw new Error('x');\n  return require(name);",
        ),
        (
            "includes of a string",
            "if (!'./a ./b'.includes(name)) throw new Error('x');\n  return require(name);",
        ),
        (
            "a regular expression that is not anchored",
            "if (!/handlers\\//.test(name)) throw new Error('x');\n  return require(name);",
        ),
        (
            "a case-insensitive regular expression",
            "if (!/^\\.\\/handlers\\//i.test(name)) throw new Error('x');\n  return require(name);",
        ),
        (
            "the exit is inside a nested function",
            "const check = () => { if (!name.startsWith('./handlers/')) throw new Error('x'); \
             };\n  return require(name);",
        ),
        (
            "the test is in a try block the load is not in",
            "try { if (!name.startsWith('./handlers/')) throw new Error('x'); } catch {}\n  \
             return require(name);",
        ),
    ] {
        assert_eq!(
            show(&format!(
                "export function load(name: any, other: any) {{\n  {body}\n}}\n"
            )),
            OPEN,
            "{why}"
        );
    }
}

#[test]
fn a_binding_that_can_change_is_not_narrowed() {
    // Assigned after the test.
    assert_eq!(
        show(
            "export function load(name: any, other: any) {\n  if \
             (!name.startsWith('./handlers/')) throw new Error('x');\n  name = other;\n  return \
             require(name);\n}\n"
        ),
        OPEN
    );
    // A `let` is not a fixed binding even when nothing assigns it.
    assert!(
        show(
            "export function load(input: any) {\n  let name = input;\n  if \
             (!name.startsWith('./handlers/')) throw new Error('x');\n  return require(name);\n}\n"
        )
        .contains("computed_specifier")
    );
    // A parameter of a function that uses `arguments`.
    let out = show(
        "export function load(name: any) {\n  if (!name.startsWith('./handlers/')) throw new \
         Error('x');\n  arguments[0] = arguments[1];\n  return require(name);\n}\n",
    );
    assert!(
        !out.is_empty() && out.lines().all(|l| l.contains("computed_specifier")),
        "{out}"
    );
}

#[test]
fn an_exit_in_front_of_a_function_declaration_says_nothing_about_its_body() {
    // `inner` is hoisted: it can be called before the test runs.
    assert_eq!(
        show(
            "export function outer(name: any) {\n  if (!name.startsWith('./handlers/')) throw new \
             Error('x');\n  return inner;\n  function inner() { return require(name); }\n}\n"
        ),
        "require dir:<unknown> !computed_specifier/param"
    );
    // A function expression exists only once control reached it.
    assert_eq!(
        show(
            "export function outer(name: any) {\n  if (!name.startsWith('./handlers/')) throw new \
             Error('x');\n  return function inner() { return require(name); };\n}\n"
        ),
        "require dir:./handlers/* !partial_specifier/unknown"
    );
}

#[test]
fn a_list_changed_by_code_that_is_not_evaluated_bounds_nothing() {
    // The list is handed to an unknown function before the test.
    assert!(
        show(
            "import { fill } from './fill';\nconst ALLOWED = ['./a'];\nfill(ALLOWED);\nexport \
             function load(name: string) {\n  if (!ALLOWED.includes(name)) throw new \
             Error('x');\n  return require(name);\n}\n"
        )
        .contains("computed_specifier")
    );
    // An object with its own `includes` is no list.
    assert_eq!(
        show(
            "const ALLOWED = { includes: () => true };\nexport function load(name: string) {\n  \
             if (!ALLOWED.includes(name)) throw new Error('x');\n  return require(name);\n}\n"
        ),
        OPEN
    );
}

#[test]
fn known_alternatives_stay_next_to_the_bound() {
    // The function also has a call in the file: its literal argument stays an
    // alternative.
    let out = show(
        "export function load(name: string) {\n  if (!name.startsWith('./handlers/')) throw new \
         Error('x');\n  return require(name);\n}\nload('./handlers/first');\n",
    );
    assert!(out.contains("./handlers/*"), "{out}");
    assert!(!out.contains("computed_specifier"), "{out}");
}
