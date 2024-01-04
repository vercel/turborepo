Setup
  $ . ${TESTDIR}/../../helpers/setup.sh
  $ VERSION=${MONOREPO_ROOT_DIR}/version.txt

Test version matches that of version.txt
  $ diff --strip-trailing-cr <(head -n 1 ${VERSION}) <(${TURBO} --version)
  1c1
  < 1.11.3
  ---
  > 1.11.3-canary.2
  [1]


TODO: resolve ambiguity
  $ ${TURBO} -v
  Turbo error: No command specified
  [1]
