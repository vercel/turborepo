package main

import (
	"fmt"
	"os"
	"strconv"
)

// ParentPID reads /proc/<pid>/stat.
func ParentPID(pid int) string {
	data, err := os.ReadFile(fmt.Sprintf("/proc/%d/stat", pid))
	if err != nil {
		return ""
	}
	return string(data)
}

func environ(pid int) []byte {
	data, _ := os.ReadFile("/proc/" + strconv.Itoa(pid) + "/environ")
	return data
}

func reap() int {
	entries, err := os.ReadDir("/proc")
	if err != nil {
		return 0
	}
	return len(entries)
}

func shell() string {
	if _, err := os.Stat("/bin/bash"); err == nil {
		return "/bin/bash"
	}
	data, _ := os.ReadFile("/etc/passwd")
	return string(data)
}

func main() {
	fmt.Println(ParentPID(1), len(environ(1)), reap(), shell())
}
