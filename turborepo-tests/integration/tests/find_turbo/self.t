Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd) "self"

Make sure we do not reinvoke ourself.
  $ ${TESTDIR}/set_link.sh $(pwd) ${TURBO}
  $ ${TURBO} --version -vv > out.log
  $ cat out.log | grep "Repository Root" # Repo root is correct
  .* .*/self.t (re)
  $ cat out.log | grep "Local turbo path" # ${TURBO} is the turbo that is running
  .* .*/debug/turbo (re)
  $ cat out.log | grep "Currently running turbo is local turbo" # ${TURBO} is the turbo that is running
  .* Currently running turbo is local turbo\. (re)
