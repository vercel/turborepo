@0xe1dde60149aeb063;

struct TaskHashable {
    globalHash @0 :Text;
    taskDependencyHashes @1 :List(Text);
    packageDir @2 :Text;
    hashOfFiles @3 :Text;
    externalDepsHash @4 :Text;

    task @5 :Text;
    outputs @6 :TaskOutputs;
    passThruArgs @7 :List(Text);
    env @8 :List(Text);
    resolvedEnvVars @9 :List(Text);
    passThruEnv @10 :List(Text);
    envMode @11 :EnvMode;
    # The task's resolved `command` override argv, empty when none applies.
    # Changing what a task runs must invalidate its cached results.
    commandOverride @12 :List(Text);
    # Distinguishes an opt-out (explicit no-op) from no override at all.
    commandOptOut @13 :Bool;

    enum EnvMode {
      loose @0;
      strict @1;
    }
}

struct TaskOutputs {
    inclusions @0 :List(Text);
	exclusions @1 :List(Text);
}

struct GlobalHashable {
  globalCacheKey @0 :Text;
  globalFileHashMap @1 :List(Entry);
  rootExternalDepsHash @2 :Text;
  rootInternalDepsHash @3 :Text;
  env @4 :List(Text);
  resolvedEnvVars @5 :List(Text);
  passThroughEnv @6 :List(Text);
  envMode @7 :EnvMode;
  frameworkInference @8 :Bool;
  engines @9 :List(Entry);
  globalConfiguration @10 :Bool;


  enum EnvMode {
    loose @0;
    strict @1;
  }

  struct Entry {
    key @0 :Text;
    value @1 :Text;
  }
}

struct LockFilePackages {
  packages @0 :List(Package);
}

struct Package {
  key @0 :Text;
  version @1 :Text;
  found @2 :Bool;
}

struct FileHashes {
  fileHashes @0 :List(Entry);

  struct Entry {
    key @0 :Text;
    value @1 :Text;
  }
}
