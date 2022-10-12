package main

// static int cgoCheck() {
//     return 2;
// }
import "C"
import (
	"fmt"
	"os"
	"unsafe"

	"github.com/vercel/turborepo/cli/internal/cmd"
)

func main() {
	// TODO(gsoltis): remove after verification
	cgoCheck := C.cgoCheck()
	fmt.Printf("CGO Check: %v\n", int(cgoCheck))
	os.Exit(cmd.RunWithArgs(os.Args[1:], turboVersion))
}

//export nativeRunWithArgs
func nativeRunWithArgs(argc C.int, argv **C.char) C.uint {
	arglen := int(argc)
	args := make([]string, arglen)
	for i, arg := range unsafe.Slice(argv, arglen) {
		args[i] = C.GoString(arg)
	}
	exitCode := cmd.RunWithArgs(args, "my-version")
	return C.uint(exitCode)
}
