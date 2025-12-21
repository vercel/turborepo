Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh framework_inference --no-install

Prove that we start with no inferred variables
  $ ${TURBO} run build --dry=json | jq -r '.tasks[].environmentVariables.inferred'
  []

Add in an inferred variable
  $ NEXT_PUBLIC_CHANGED=true ${TURBO} run build --dry=json | jq -c '.tasks[].environmentVariables.inferred'
  ["NEXT_PUBLIC_CHANGED=b5bea41b6c623f7c09f1bf24dcae58ebab3c0cdd90ad966bc43a45b44867e12b"]

Baseline for excluding via TURBO_CI_VENDOR_ENV_KEY
  $ NEXT_PUBLIC_CHANGED=true NEXT_PUBLIC_IGNORED_VALUE=true ${TURBO} run build --dry=json | jq -c '.tasks[].environmentVariables.inferred'
  ["NEXT_PUBLIC_CHANGED=b5bea41b6c623f7c09f1bf24dcae58ebab3c0cdd90ad966bc43a45b44867e12b","NEXT_PUBLIC_IGNORED_VALUE=b5bea41b6c623f7c09f1bf24dcae58ebab3c0cdd90ad966bc43a45b44867e12b"]

Exclude a variable using TURBO_CI_VENDOR_ENV_KEY
  $ NEXT_PUBLIC_CHANGED=true NEXT_PUBLIC_IGNORED_VALUE=true TURBO_CI_VENDOR_ENV_KEY=NEXT_PUBLIC_IGNORED_ ${TURBO} run build --dry=json | jq -c '.tasks[].environmentVariables.inferred'
  ["NEXT_PUBLIC_CHANGED=b5bea41b6c623f7c09f1bf24dcae58ebab3c0cdd90ad966bc43a45b44867e12b"]

Switch off framework inference and we no longer include inferred variables.
  $ NEXT_PUBLIC_CHANGED=true ${TURBO} run build --framework-inference=false --dry=json | jq -r '.tasks[].environmentVariables.inferred'
  []

Confirm that the right values appear in the run summary when framework inference is on.
  $ ${TURBO} run build --framework-inference=true --dry=json > output.json
  $ cat output.json | jq -r '.frameworkInference'
  true
  $ cat output.json | jq -r '.tasks[].framework'
  nextjs

Confirm that the right values appear in the run summary when framework inference is off.
  $ ${TURBO} run build --framework-inference=false --dry=json > output.json
  $ cat output.json | jq -r '.frameworkInference'
  false
  $ cat output.json | jq -r '.tasks[].framework'
  

Exclude framework-inferred variables using negative wildcard in env key.
First, update turbo.json to add env exclusion pattern.
  $ cat turbo.json | jq '.tasks.build.env = ["!NEXT_PUBLIC_*"]' > turbo.json.tmp && mv turbo.json.tmp turbo.json
  $ cat turbo.json
  {
    "$schema": "https://turborepo.com/schema.json",
    "globalPassThroughEnv": [],
    "tasks": {
      "build": {
        "env": [
          "!NEXT_PUBLIC_*"
        ]
      }
    }
  }

Now verify that framework-inferred vars are excluded when negative wildcard is present.
  $ NEXT_PUBLIC_CHANGED=true ${TURBO} run build --dry=json | jq -c '.tasks[].environmentVariables.inferred'
  []

The framework is still detected even with exclusion.
  $ NEXT_PUBLIC_CHANGED=true ${TURBO} run build --dry=json | jq -r '.tasks[].framework'
  nextjs

Test globalEnv exclusion also applies to framework-inferred vars.
Reset turbo.json and use globalEnv instead.
  $ cat > turbo.json << 'EOF'
  > {
  >   "$schema": "https://turborepo.com/schema.json",
  >   "globalEnv": ["!NEXT_PUBLIC_*"],
  >   "globalPassThroughEnv": [],
  >   "tasks": {
  >     "build": {}
  >   }
  > }
  > EOF

Verify globalEnv exclusions also apply to framework-inferred variables.
  $ NEXT_PUBLIC_CHANGED=true ${TURBO} run build --dry=json | jq -c '.tasks[].environmentVariables.inferred'
  []

