#!/bin/bash

# We set this explicitly to stream, so we can lock into to streaming logs (i.e. not "auto") behavior.
#
# We do this because when these tests are invoked in CI (through .github/actions/test.yml), they will
# inherit the GITHUB_ACTIONS=true env var, and each of the `turbo run` invocations into behavior
# we do not want. Since prysk mainly tests log output, this extra behavior will break all the tests
# and can be unpredictable over time, as we make "auto" do more magic.
#
# Note: since these tests are invoked _through_ turbo, the ideal setup would be to pass --env-mode=strict
# so we can prevent the `GITHUB_ACTIONS` env var from being passed down here from the top level turbo.
# But as of now, this breaks our tests (and I'm not sure why). If we make that work, we can remove this
# explicit locking of log order. See PR attempt here: https://github.com/vercel/turbo/pull/5324
export TURBO_LOG_ORDER=stream

if [ "$1" = "" ]; then
  .cram_env/bin/prysk --shell="$(which bash)" tests
else
  .cram_env/bin/prysk --shell="$(which bash)" "tests/$1"
fi
