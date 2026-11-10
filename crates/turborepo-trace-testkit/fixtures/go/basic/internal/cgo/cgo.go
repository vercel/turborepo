package cgo

// #cgo CFLAGS: -O2
// #include <stdlib.h>
// #include "native.h"
import "C"

// Native calls into C.
func Native() int { return int(C.native()) }
