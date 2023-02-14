Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd)

Expect warnings
  $ ${TURBO} build -v --dry > /dev/null
  [-0-9:.TWZ+]+ \[INFO]  turbo: skipping turbod since we appear to be in a non-interactive context (re)
  [0-9]{4}/[0-9]{2}/[0-9]{2} [-0-9:.TWZ+]+ \[WARNING] Using an absolute path in "outputs" \(/another/absolute/path\) will not work and will be an error in a future version (re)
  [0-9]{4}/[0-9]{2}/[0-9]{2} [-0-9:.TWZ+]+ \[WARNING] Using an absolute path in "inputs" \(/some/absolute/path\) will not work and will be an error in a future version (re)
  [0-9]{4}/[0-9]{2}/[0-9]{2} [-0-9:.TWZ+]+ \[WARNING] Using an absolute path in "globalDependencies" \(/an/absolute/path\) will not work and will be an error in a future version (re)
