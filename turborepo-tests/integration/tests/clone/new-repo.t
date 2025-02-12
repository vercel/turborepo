Setup
  $ . ${TESTDIR}/../../helpers/setup.sh

Create a repo
  $ mkdir my-repo
  $ cd my-repo
  $ git init --quiet
  $ echo "Hello World" > README.md
  $ git add README.md
  $ git commit -m "Initial commit" --quiet

Make sure we allow partial clones
  $ git config uploadpack.allowFilter true
  $ cd ..

Clone repo with `--ci`
  $ ${TURBO} clone file://$(pwd)/my-repo my-repo-treeless --ci
  $ cd my-repo-treeless
Assert it's a treeless clone
  $ git config remote.origin.partialclonefilter
  $ cd ..

Clone repo with `--local`
  $ ${TURBO} clone file://$(pwd)/my-repo my-repo-blobless --local
  $ cd my-repo-blobless
Assert it's a blobless clone
  $ git config remote.origin.partialclonefilter
