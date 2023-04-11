Setup
  $ . ${TESTDIR}/../../helpers/setup.sh
  $ . ${TESTDIR}/_helpers/setup_monorepo.sh $(pwd) framework_inference

Set the env vars
  $ export NEXT_PUBLIC_HELLO=hellofromnext
  $ export VITE_HELLO=hellofromvite
  $ export VUE_APP_HELLO=hellofromvue

Check next
  $ rm -rf .turbo/runs
  $ ${TURBO} build --filter=next-app --summarize | grep "next-app:build: hello from "
  next-app:build: hello from hellofromnext
  $ cat .turbo/runs/*.json | jq -r '.tasks[0].environmentVariables.inferred'
  [
    "NEXT_PUBLIC_HELLO=7719a612cd8fb5d22a8207ee9dba3e55d2f7d712e3dfd40af2d9080a545ff427"
  ]

Check next + strict
  $ rm -rf .turbo/runs
  $ ${TURBO} build --filter=next-app --experimental-env-mode=strict --summarize | grep "next-app:build: hello from "
  next-app:build: hello from 
  $ cat .turbo/runs/*.json | jq -r '.tasks[0].environmentVariables.inferred'
  []

Check vite
  $ rm -rf .turbo/runs
  $ ${TURBO} build --filter=vite-app --summarize | grep "vite-app:build: hello from "
  vite-app:build: hello from hellofromvite
  $ cat .turbo/runs/*.json | jq -r '.tasks[0].environmentVariables.inferred'
  [
    "VITE_HELLO=7e153fd3e8ad33597a4ecf6d1a96a91dfba32cbb695e2729228cb9af8f615567"
  ]

Check vite + strict
  $ rm -rf .turbo/runs
  $ ${TURBO} build --filter=vite-app --experimental-env-mode=strict --summarize | grep "vite-app:build: hello from "
  vite-app:build: hello from 
  $ cat .turbo/runs/*.json | jq -r '.tasks[0].environmentVariables.inferred'
  []

Check vue
  $ rm -rf .turbo/runs
  $ ${TURBO} build --filter=vue-app --summarize | grep "vue-app:build: hello from "
  vue-app:build: hello from hellofromvue
  $ cat .turbo/runs/*.json | jq -r '.tasks[0].environmentVariables.inferred'
  [
    "VUE_APP_HELLO=429b02cec7c8171389c5d0eaff1ef36b47e418bb96050979f9cf2bea34c07539"
  ]

Check vue + strict
  $ rm -rf .turbo/runs
  $ ${TURBO} build --filter=vue-app --experimental-env-mode=strict --summarize | grep "vue-app:build: hello from "
  vue-app:build: hello from 
  $ cat .turbo/runs/*.json | jq -r '.tasks[0].environmentVariables.inferred'
  []
