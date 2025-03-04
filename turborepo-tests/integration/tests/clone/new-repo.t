Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh

Create a repo
  $ mkdir my-repo
  $ cd my-repo
  $ git init --quiet
  $ echo "Hello World" > README.md
  $ git add README.md
  $ git config user.email "test@example.com"
  $ git config user.name "Test"
  $ git commit -m "Initial commit" --quiet

Make sure we allow partial clones
  $ git config uploadpack.allowFilter true
  $ cd ..

Clone repo with `--ci`
  $ ${TURBO} clone file://$(pwd)/my-repo my-repo-treeless --ci
  Cloning into 'my-repo-treeless'...
  $ cd my-repo-treeless
Assert it's a treeless clone
  $ git config remote.origin.partialclonefilter
  tree:0
  $ cd ..

Clone repo with `--local`
  $ ${TURBO} clone file://$(pwd)/my-repo my-repo-blobless --local
  Cloning into 'my-repo-blobless'...
  $ cd my-repo-blobless
Assert it's a blobless clone
  $ git config remote.origin.partialclonefilter
  blob:none
