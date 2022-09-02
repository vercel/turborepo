// #include <stdio.h>
#include "libturbo.h"

int main(int argc, char **argv) {
  char **stripped_args;
  if (argc == 1) {
    stripped_args = NULL;
  } else {
    stripped_args = &argv[1];
  }
  int exit_code = nativeRunWithArgs(argc - 1, stripped_args) ;
  return exit_code;
}
