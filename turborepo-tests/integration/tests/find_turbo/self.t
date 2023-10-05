Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd) "self"

Make sure we do not reinvoke ourself.
  $ ${TESTDIR}/set_link.sh $(pwd) ${TURBO}
  $ ${TURBO} --version -vv
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Global turbo version: .* (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Repository Root: .*/self.t (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Local turbo path: .*/debug/turbo (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Local turbo version: 1.0.0 (re)
  [-0-9:.TWZ+]+ \[DEBUG] turborepo_lib::shim: Currently running turbo is local turbo. (re)
  .* (re)
