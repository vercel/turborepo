turbostub.exe

This stub is generated with https://github.com/mehulkar/turbostub.exe and should run on Windows.

It is the equivalent of the bash script that is used as the stub in the darwin and linux variants
in the node_modules in these find_turbo fixtures.

```
#/!/bin/bash
echo $@
```

Rather than duplicate this 57kb exe in every stub, we add some setup code to copy this over to
each stub before the test execution.
