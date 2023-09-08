# `turborepo-tests/examples`

The tests in this directory exercise the `examples/` directory. They validate
that when someone uses those examples (likely via `npx create-turbo -e <example>`),
the example works.

These tests do _not_ use a local `turbo` build, they use the version installed in each
of the `examples/*` directories in their respective `package.json`s.
