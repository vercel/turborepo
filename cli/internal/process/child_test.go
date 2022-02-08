package process

/**
 * Code in this file is based on the source code at
 * https://github.com/hashicorp/consul-template/tree/3ea7d99ad8eff17897e0d63dac86d74770170bb8/child/child_test.go
 *
 * Major changes include supporting api changes in child.go and removing
 * tests for reloading, which was removed in child.go
 */

import (
	"io/ioutil"
	"os"
	"os/exec"
	"strings"
	"testing"
	"time"

	"github.com/hashicorp/go-gatedio"
	"github.com/hashicorp/go-hclog"
)

const fileWaitSleepDelay = 150 * time.Millisecond

func testChild(t *testing.T) *Child {
	cmd := exec.Command("echo", "hello", "world")
	cmd.Stdout = ioutil.Discard
	cmd.Stderr = ioutil.Discard
	c, err := newChild(NewInput{
		Cmd:         cmd,
		KillSignal:  os.Kill,
		KillTimeout: 2 * time.Second,
		Splay:       0 * time.Second,
		Logger:      hclog.Default(),
	})
	if err != nil {
		t.Fatal(err)
	}
	return c
}

func TestNew(t *testing.T) {

	stdin := gatedio.NewByteBuffer()
	stdout := gatedio.NewByteBuffer()
	stderr := gatedio.NewByteBuffer()
	command := "echo"
	args := []string{"hello", "world"}
	env := []string{"a=b", "c=d"}
	killSignal := os.Kill
	killTimeout := fileWaitSleepDelay
	splay := fileWaitSleepDelay

	cmd := exec.Command(command, args...)
	cmd.Stdin = stdin
	cmd.Stderr = stderr
	cmd.Stdout = stdout
	cmd.Env = env
	c, err := newChild(NewInput{
		Cmd:         cmd,
		KillSignal:  killSignal,
		KillTimeout: killTimeout,
		Splay:       splay,
		Logger:      hclog.Default(),
	})
	if err != nil {
		t.Fatal(err)
	}

	if c.killSignal != killSignal {
		t.Errorf("expected %q to be %q", c.killSignal, killSignal)
	}

	if c.killTimeout != killTimeout {
		t.Errorf("expected %q to be %q", c.killTimeout, killTimeout)
	}

	if c.splay != splay {
		t.Errorf("expected %q to be %q", c.splay, splay)
	}

	if c.stopCh == nil {
		t.Errorf("expected %#v to be", c.stopCh)
	}
}

func TestExitCh_noProcess(t *testing.T) {

	c := testChild(t)
	ch := c.ExitCh()
	if ch != nil {
		t.Errorf("expected %#v to be nil", ch)
	}
}

func TestExitCh(t *testing.T) {

	c := testChild(t)
	if err := c.Start(); err != nil {
		t.Fatal(err)
	}
	println("Started")
	defer c.Stop()

	ch := c.ExitCh()
	if ch == nil {
		t.Error("expected ch to exist")
	}
}

func TestPid_noProcess(t *testing.T) {

	c := testChild(t)
	pid := c.Pid()
	if pid != 0 {
		t.Errorf("expected %q to be 0", pid)
	}
}

func TestPid(t *testing.T) {

	c := testChild(t)
	if err := c.Start(); err != nil {
		t.Fatal(err)
	}
	defer c.Stop()

	pid := c.Pid()
	if pid == 0 {
		t.Error("expected pid to not be 0")
	}
}

func TestStart(t *testing.T) {

	c := testChild(t)

	// Set our own reader and writer so we can verify they are wired to the child.
	stdin := gatedio.NewByteBuffer()
	stdout := gatedio.NewByteBuffer()
	stderr := gatedio.NewByteBuffer()
	// Custom env and command
	env := []string{"a=b", "c=d"}
	cmd := exec.Command("env")
	cmd.Stdin = stdin
	cmd.Stdout = stdout
	cmd.Stderr = stderr
	cmd.Env = env
	c.cmd = cmd

	if err := c.Start(); err != nil {
		t.Fatal(err)
	}
	defer c.Stop()

	select {
	case <-c.ExitCh():
	case <-time.After(fileWaitSleepDelay):
		t.Fatal("process should have exited")
	}

	output := stdout.String()
	for _, envVar := range env {
		if !strings.Contains(output, envVar) {
			t.Errorf("expected to find %q in %q", envVar, output)
		}
	}
}

func TestKill_noSignal(t *testing.T) {

	c := testChild(t)
	c.cmd = exec.Command("sh", "-c", "while true; do sleep 0.2; done")
	c.killTimeout = 20 * time.Millisecond
	c.killSignal = nil

	if err := c.Start(); err != nil {
		t.Fatal(err)
	}
	defer c.Stop()

	// For some reason bash doesn't start immediately
	time.Sleep(fileWaitSleepDelay)

	c.Kill()

	// Give time for the file to flush
	time.Sleep(fileWaitSleepDelay)

	if c.cmd != nil {
		t.Errorf("expected cmd to be nil")
	}
}
