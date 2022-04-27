package cmdutil

// Error is a specific error that is returned by the command to specify the exit code
type Error struct {
	ExitCode int
	Err      error
}

func (e *Error) Error() string { return e.Err.Error() }

// BasicError is an empty error that is returned by the command
type BasicError struct{}

func (e *BasicError) Error() string { return "basic error" }
