using Go = import "/go.capnp";

@0xe1dde60149aeb063;

$Go.package("proto");
$Go.import("proto");

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
    dotEnv @12 :List(Text);

    enum EnvMode {
      infer @0;
      loose @1;
      strict @2;
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
  env @3 :List(Text);
  resolvedEnvVars @4 :List(Text);
  passThroughEnv @5 :List(Text);
  envMode @6 :EnvMode;
  frameworkInference @7 :Bool;
  dotEnv @8 :List(Text);


  enum EnvMode {
    infer @0;
    loose @1;
    strict @2;
  }

  struct Entry {
    key @0 :Text;
    value @1 :Text;
  }
}
