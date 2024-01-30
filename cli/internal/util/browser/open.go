package browser

import (
	"fmt"
	"os/exec"
	"runtime"
)

// OpenBrowser attempts to interactively open a browser window at the given URL
func OpenBrowser(url string) error {
	var err error

	switch runtime.GOOS {
	case "linux":
		if posixBinExists("wslview") {
			err = exec.Command("wslview", url).Start()
		} else {
			err = exec.Command("xdg-open", url).Start()
		}
	case "windows":
		err = exec.Command("rundll32", "url.dll,FileProtocolHandler", url).Start()
	case "darwin":
		err = exec.Command("open", url).Start()
	default:
		err = fmt.Errorf("unsupported platform")
	}
	if err != nil {
		return err
	}
	return nil
}

func posixBinExists(bin string) bool {
	err := exec.Command("which", bin).Run()
	// we mostly don't care what the error is, it suggests the binary is not usable
	return err == nil
}
