export const basicPipeline = {
  pipeline: {
    test: {
      dependsOn: ["^build"],
      outputs: [],
    },
    lint: {
      inputs: ["build.js", "lint.js"],
      outputs: [],
    },
    build: {
      dependsOn: ["^build"],
      outputs: ["dist/**", "!dist/cache/**"],
    },
    "//#build": {
      dependsOn: [],
      outputs: ["dist/**"],
      inputs: ["rootbuild.js"],
    },
    "//#special": {
      dependsOn: ["^build"],
      outputs: ["dist/**"],
      inputs: [],
    },
    "//#args": {
      dependsOn: [],
      outputs: [],
    },
  },
  globalEnv: ["GLOBAL_ENV_DEPENDENCY"],
};

export const prunePipeline = {
  ...basicPipeline,
  pipeline: {
    ...basicPipeline.pipeline,
    // add some package specific pipeline tasks to test pruning
    "a#build": {
      outputs: ["dist/**", "!dist/cache/**"],
    },
    "c#build": {
      outputs: ["dist/**", "!dist/cache/**"],
    },
  },
};

export const explicitPrunePipeline = {
  ...basicPipeline,
  pipeline: {
    ...basicPipeline.pipeline,
    // add some package specific pipeline tasks to test pruning
    "a#build": {
      dependsOn: ["b#build"],
      outputs: ["dist/**", "!dist/cache/**"],
    },
    "b#build": {
      outputs: ["dist/**", "!dist/cache/**"],
    },
    "c#build": {
      outputs: ["dist/**", "!dist/cache/**"],
    },
  },
};
