//go:build windows
// +build windows

package connector

import "syscall"

// getSysProcAttrs returns the platform-specific attributes we want to
// use while forking the daemon process. Currently this is limited to
// forcing a new process group
func getSysProcAttrs() *syscall.SysProcAttr {
	return &syscall.SysProcAttr{
		CreationFlags: syscall.CREATE_NEW_PROCESS_GROUP,
	}
}
