//go:build !windows
// +build !windows

package process

/**
 * Code in this file is based on the source code at
 * https://github.com/hashicorp/consul-template/tree/3ea7d99ad8eff17897e0d63dac86d74770170bb8/child/child_test.go
 *
 * Tests in this file use signals or pgid features not available on windows
 */

import (
	"os/exec"
	"syscall"
	"testing"
	"time"

	"github.com/hashicorp/go-gatedio"
)

func TestSignal(t *testing.T) {

	c := testChild(t)
	cmd := exec.Command("sh", "-c", "trap 'echo one; exit' USR1; while true; do sleep 0.2; done")
	c.cmd = cmd

	out := gatedio.NewByteBuffer()
	c.cmd.Stdout = out

	if err := c.Start(); err != nil {
		t.Fatal(err)
	}
	defer c.Stop()

	// For some reason bash doesn't start immediately
	time.Sleep(fileWaitSleepDelay)

	if err := c.Signal(syscall.SIGUSR1); err != nil {
		t.Fatal(err)
	}

	// Give time for the file to flush
	time.Sleep(fileWaitSleepDelay)

	expected := "one\n"
	if out.String() != expected {
		t.Errorf("expected %q to be %q", out.String(), expected)
	}
}

func TestStop_childAlreadyDead(t *testing.T) {
	c := testChild(t)
	c.cmd = exec.Command("sh", "-c", "exit 1")
	c.splay = 100 * time.Second
	c.killSignal = syscall.SIGTERM

	if err := c.Start(); err != nil {
		t.Fatal(err)
	}

	// For some reason bash doesn't start immediately
	time.Sleep(fileWaitSleepDelay)

	killStartTime := time.Now()
	c.Stop()
	killEndTime := time.Now()

	if killEndTime.Sub(killStartTime) > fileWaitSleepDelay {
		t.Error("expected not to wait for splay")
	}
}

func TestSignal_noProcess(t *testing.T) {

	c := testChild(t)
	if err := c.Signal(syscall.SIGUSR1); err != nil {
		// Just assert there is no error
		t.Fatal(err)
	}
}

func TestKill_signal(t *testing.T) {

	c := testChild(t)
	cmd := exec.Command("sh", "-c", "trap 'echo one; exit' USR1; while true; do sleep 0.2; done")
	c.killSignal = syscall.SIGUSR1

	out := gatedio.NewByteBuffer()
	cmd.Stdout = out
	c.cmd = cmd

	if err := c.Start(); err != nil {
		t.Fatal(err)
	}
	defer c.Stop()

	// For some reason bash doesn't start immediately
	time.Sleep(fileWaitSleepDelay)

	c.Kill()

	// Give time for the file to flush
	time.Sleep(fileWaitSleepDelay)

	expected := "one\n"
	if out.String() != expected {
		t.Errorf("expected %q to be %q", out.String(), expected)
	}
}

func TestKill_noProcess(t *testing.T) {
	c := testChild(t)
	c.killSignal = syscall.SIGUSR1
	c.Kill()
}

func TestStop_noWaitForSplay(t *testing.T) {
	c := testChild(t)
	c.cmd = exec.Command("sh", "-c", "trap 'echo one; exit' USR1; while true; do sleep 0.2; done")
	c.splay = 100 * time.Second
	c.killSignal = syscall.SIGUSR1

	out := gatedio.NewByteBuffer()
	c.cmd.Stdout = out

	if err := c.Start(); err != nil {
		t.Fatal(err)
	}

	// For some reason bash doesn't start immediately
	time.Sleep(fileWaitSleepDelay)

	killStartTime := time.Now()
	c.StopImmediately()
	killEndTime := time.Now()

	expected := "one\n"
	if out.String() != expected {
		t.Errorf("expected %q to be %q", out.String(), expected)
	}

	if killEndTime.Sub(killStartTime) > fileWaitSleepDelay {
		t.Error("expected not to wait for splay")
	}
}

func TestSetpgid(t *testing.T) {
	t.Run("true", func(t *testing.T) {
		c := testChild(t)
		c.cmd = exec.Command("sh", "-c", "while true; do sleep 0.2; done")
		// default, but to be explicit for the test
		c.setpgid = true

		if err := c.Start(); err != nil {
			t.Fatal(err)
		}
		defer c.Stop()

		// when setpgid is true, the pid and gpid should be the same
		gpid, err := syscall.Getpgid(c.Pid())
		if err != nil {
			t.Fatal("Getpgid error:", err)
		}

		if c.Pid() != gpid {
			t.Fatal("pid and gpid should match")
		}
	})
	t.Run("false", func(t *testing.T) {
		c := testChild(t)
		c.cmd = exec.Command("sh", "-c", "while true; do sleep 0.2; done")
		c.setpgid = false

		if err := c.Start(); err != nil {
			t.Fatal(err)
		}
		defer c.Stop()

		// when setpgid is true, the pid and gpid should be the same
		gpid, err := syscall.Getpgid(c.Pid())
		if err != nil {
			t.Fatal("Getpgid error:", err)
		}

		if c.Pid() == gpid {
			t.Fatal("pid and gpid should NOT match")
		}
	})
}
