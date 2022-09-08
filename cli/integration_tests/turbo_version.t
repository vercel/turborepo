Setup
  $ . ${TESTDIR}/setup.sh

Test version
  $ ${TURBO} --version
  (?P<major>0|[1-9]\d*)\.(?P<minor>0|[1-9]\d*)\.(?P<patch>0|[1-9]\d*)(?:-(?P<prerelease>(?:0|[1-9]\d*|\d*[a-zA-Z-][0-9a-zA-Z-]*)(?:\.(?:0|[1-9]\d*|\d*[a-zA-Z-][0-9a-zA-Z-]*))*))?(?:\+(?P<buildmetadata>[0-9a-zA-Z-]+(?:\.[0-9a-zA-Z-]+)*))?$ (re)

Semver Regex source: https://semver.org/#is-there-a-suggested-regular-expression-regex-to-check-a-semver-string
TODO: resolve ambiguity
$ ${TURBO} -v
