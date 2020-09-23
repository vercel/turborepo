package main

/*
#include <stdlib.h>
#include <ftdi.h>
#include <libusb.h>
#cgo pkg-config: libftdi1 libusb-1.0
*/
import "C"

import (
	"fmt"
)

var (
	version string
)

func main() {
	fmt.Printf("%s\n", version)
}
