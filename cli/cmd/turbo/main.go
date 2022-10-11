package main

// static int cgoCheck() {
//     return 2;
// }
import "C"
import (
	"encoding/json"
	"fmt"
	"os"
	"unsafe"

	"github.com/vercel/turborepo/cli/internal/cmd"
)

func main() {
	// TODO(gsoltis): remove after verification
	cgoCheck := C.cgoCheck()
	fmt.Printf("CGO Check: %v\n", int(cgoCheck))
	os.Exit(cmd.RunWithArgs(os.Args[1:], turboVersion, &cmd.TurboState{}))
}

//export nativeRunWithArgs
func nativeRunWithArgs(argc C.int, argv **C.char, turboStateString string) C.uint {
	arglen := int(argc)
	args := make([]string, arglen)
	for i, arg := range unsafe.Slice(argv, arglen) {
		args[i] = C.GoString(arg)
	}

	turboState := cmd.TurboState{}
	if turboStateString != "" {
		err := json.Unmarshal([]byte(turboStateString), &turboState)
		if err != nil {
			fmt.Printf("Error unmarshaling json: %v\n", err)
			return C.uint(2)
		}
	}

	exitCode := cmd.RunWithArgs(args, "my-version", &turboState)
	return C.uint(exitCode)
}
