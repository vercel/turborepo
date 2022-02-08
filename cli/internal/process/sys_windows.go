//go:build windows
// +build windows

package process

/**
 * Code in this file is based on the source code at
 * https://github.com/hashicorp/consul-template/tree/3ea7d99ad8eff17897e0d63dac86d74770170bb8/child/sys_windows.go
 */

import "os/exec"

func setSetpgid(cmd *exec.Cmd, value bool) {}

func processNotFoundErr(err error) bool {
	return false
}
