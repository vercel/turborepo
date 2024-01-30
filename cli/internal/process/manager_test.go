package process

import (
	"errors"
	"os/exec"
	"sync"
	"testing"
	"time"

	"github.com/hashicorp/go-gatedio"
	"github.com/hashicorp/go-hclog"
)

func newManager() *Manager {
	return NewManager(hclog.Default())
}

func TestExec_simple(t *testing.T) {
	mgr := newManager()

	out := gatedio.NewByteBuffer()
	cmd := exec.Command("env")
	cmd.Stdout = out

	err := mgr.Exec(cmd)
	if err != nil {
		t.Errorf("expected %q to be nil", err)
	}

	output := out.String()
	if output == "" {
		t.Error("expected output from running 'env', got empty string")
	}
}

func TestClose(t *testing.T) {
	mgr := newManager()

	wg := sync.WaitGroup{}
	tasks := 4
	errors := make([]error, tasks)
	start := time.Now()
	for i := 0; i < tasks; i++ {
		wg.Add(1)
		go func(index int) {
			cmd := exec.Command("sleep", "0.5")
			err := mgr.Exec(cmd)
			if err != nil {
				errors[index] = err
			}
			wg.Done()
		}(i)
	}
	// let processes kick off
	time.Sleep(50 * time.Millisecond)
	mgr.Close()
	end := time.Now()
	wg.Wait()
	duration := end.Sub(start)
	if duration >= 500*time.Millisecond {
		t.Errorf("expected to close, total time was %q", duration)
	}
	for _, err := range errors {
		if err != ErrClosing {
			t.Errorf("expected manager closing error, found %q", err)
		}
	}
}

func TestClose_alreadyClosed(t *testing.T) {
	mgr := newManager()
	mgr.Close()

	// repeated closing does not error
	mgr.Close()

	err := mgr.Exec(exec.Command("sleep", "1"))
	if err != ErrClosing {
		t.Errorf("expected manager closing error, found %q", err)
	}
}

func TestExitCode(t *testing.T) {
	mgr := newManager()

	err := mgr.Exec(exec.Command("ls", "doesnotexist"))
	exitErr := &ChildExit{}
	if !errors.As(err, &exitErr) {
		t.Errorf("expected a ChildExit err, got %q", err)
	}
	if exitErr.ExitCode == 0 {
		t.Error("expected non-zero exit code , got 0")
	}
}
