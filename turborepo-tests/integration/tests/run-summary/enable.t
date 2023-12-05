Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh

# Delete all run summaries to start
  $ rm -rf .turbo/runs

# Tests
| env var | flag    | summary? |
| ------- | ------- | -------- |
| true    | missing | yes      |
| true    | true    | yes      |
| true    | false   | no       |
| true    | novalue | yes      |

| false   | missing | no       |
| false   | true    | yes      |
| false   | false   | no       |
| false   | novalue | yes      |

| missing | missing | no       |
| missing | true    | yes      |
| missing | false   | no       |
| missing | novalue | yes      |


# env var=true, missing flag: yes
  $ rm -rf .turbo/runs
  $ TURBO_RUN_SUMMARY=true ${TURBO} run build > /dev/null
  $ /bin/ls .turbo/runs/*.json | wc -l
  \s*1 (re)
# env var=true, --flag=true: yes
  $ rm -rf .turbo/runs
  $ TURBO_RUN_SUMMARY=true ${TURBO} run build --summarize=true > /dev/null
  $ /bin/ls .turbo/runs/*.json | wc -l
  \s*1 (re)
# env var=true, --flag=false: no
  $ rm -rf .turbo/runs
  $ TURBO_RUN_SUMMARY=true ${TURBO} run build --summarize=false > /dev/null
  $ test -d .turbo/runs
  [1]
# env var=true, --flag (no value): yes
  $ rm -rf .turbo/runs
  $ TURBO_RUN_SUMMARY=true ${TURBO} run build --summarize > /dev/null
  $ /bin/ls .turbo/runs/*.json | wc -l
  \s*1 (re)

# env var=false, missing flag, no
  $ rm -rf .turbo/runs
  $ TURBO_RUN_SUMMARY=false ${TURBO} run build > /dev/null
  $ test -d .turbo/runs
  [1]
# env var=false, --flag=true: yes
  $ rm -rf .turbo/runs
  $ TURBO_RUN_SUMMARY=false ${TURBO} run build --summarize=true > /dev/null
  $ /bin/ls .turbo/runs/*.json | wc -l
  \s*1 (re)
# env var=false, --flag=false: no
  $ rm -rf .turbo/runs
  $ TURBO_RUN_SUMMARY=false ${TURBO} run build --summarize=false > /dev/null
  $ test -d .turbo/runs
  [1]
# env var=false, --flag (no value): yes
  $ rm -rf .turbo/runs
  $ TURBO_RUN_SUMMARY=false ${TURBO} run build --summarize > /dev/null
  $ /bin/ls .turbo/runs/*.json | wc -l
  \s*1 (re)

# missing env var, missing flag: no
  $ rm -rf .turbo/runs
  $ ${TURBO} run build > /dev/null
  $ test -d .turbo/runs
  [1]
# missing env var, --flag=true: yes
  $ rm -rf .turbo/runs
  $ ${TURBO} run build --summarize=true > /dev/null
  $ /bin/ls .turbo/runs/*.json | wc -l
  \s*1 (re)
# missing env var, --flag=false: no
  $ rm -rf .turbo/runs
  $ ${TURBO} run build --summarize=false > /dev/null
  $ test -d .turbo/runs
  [1]
# missing env var, --flag (no value): yes
  $ rm -rf .turbo/runs
  $ ${TURBO} run build --summarize > /dev/null
  $ /bin/ls .turbo/runs/*.json | wc -l
  \s*1 (re)
