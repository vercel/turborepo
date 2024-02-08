Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh

Use our custom turbo config which has foo.txt as an input to the build command
  $ . ${TESTDIR}/../../../helpers/replace_turbo_json.sh $(pwd) "gitignored-inputs.json"

Create a internal.txt for the util package and add it to gitignore
This field is already part of our turbo config.
  $ echo "hello world" >> packages/util/internal.txt
  $ echo "packages/util/internal.txt" >> ${PWD}/.gitignore
  $ if [[ "$OSTYPE" == "msys" ]]; then dos2unix --quiet packages/util/internal.txt; fi
  $ git add . && git commit --quiet -m  "add internal.txt"

Some helper functions to parse the summary file
  $ source "$TESTDIR/../../../helpers/run_summary.sh"

Just run the util package, it's simpler
  $ ${TURBO} run build --filter=util --output-logs=hash-only --summarize | grep "util:build: cache"
  util:build: cache miss, executing 2f6ab59379ddcb93

  $ FIRST=$(/bin/ls .turbo/runs/*.json | head -n1)
  $ echo $(getSummaryTaskId $FIRST "util#build") | jq -r '.inputs."internal.txt"'
  3b18e512dba79e4c8300dd08aeb37f8e728b8dad

Cleanup the runs folder so we don't have to select the correct file for the second run
  $ rm -rf .turbo/runs

Change the content of internal.txt
  $ echo "changed!" >> packages/util/internal.txt

Hash does not change, because it is gitignored
  $ ${TURBO} run build --filter=util --output-logs=hash-only --summarize | grep "util:build: cache"
  util:build: cache miss, executing a26c95f27f26f89c

The internal.txt hash should be different from the one before
  $ SECOND=$(/bin/ls .turbo/runs/*.json | head -n1)
  $ echo $(getSummaryTaskId $SECOND "util#build") | jq -r '.inputs."internal.txt"'
  fe9ca9502b0cfe311560aa43d953a88b112609ce
