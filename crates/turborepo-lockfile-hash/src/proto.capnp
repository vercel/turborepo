@0xe1dde60149aeb063;

struct LockFilePackages {
  packages @0 :List(Package);
}

struct Package {
  key @0 :Text;
  version @1 :Text;
  found @2 :Bool;
}
