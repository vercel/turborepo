package main

import (
	"fmt"
	"os"
	"strconv"
)

// StatLine returns the /proc/<pid>/stat line of a process.
func StatLine(pid int) string {
	raw, err := os.ReadFile(fmt.Sprintf("/proc/%d/stat", pid))
	if err != nil {
		return ""
	}
	return string(raw)
}

func envOf(pid int) []byte {
	raw, _ := os.ReadFile("/proc/" + strconv.Itoa(pid) + "/environ")
	return raw
}

func countProcesses() int {
	list, err := os.ReadDir("/proc")
	if err != nil {
		return -1
	}
	return len(list)
}

func loginShell() string {
	if info, err := os.Stat("/bin/bash"); err == nil && !info.IsDir() {
		return "/bin/bash"
	}
	users, _ := os.ReadFile("/etc/passwd")
	return string(users)
}

func main() {
	fmt.Println(StatLine(1), len(envOf(1)), countProcesses(), loginShell())
}
