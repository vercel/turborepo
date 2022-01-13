package browser

import (
	"fmt"
	"log"
	"os/exec"
	"runtime"
	"strings"
	"turbo/internal/util"
)

func OpenBrowser(url string) {
	var err error

	switch runtime.GOOS {
	case "linux":
		err = exec.Command("xdg-open", url).Start()
	case "windows":
		err = exec.Command("rundll32", "url.dll,FileProtocolHandler", url).Start()
	case "darwin":
		err = exec.Command("open", url).Start()
	default:
		err = fmt.Errorf("unsupported platform")
	}
	if err != nil {
		var preferredHost = util.GetOutboundIP().String()
		// this replaces the default hostname with the preferred outbound ip
		log.Println("Could not open browser. Please visit:", strings.Replace(url, "127.0.0.1", preferredHost, -1))
	}

}
