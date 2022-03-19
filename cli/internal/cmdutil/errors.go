package cmdutil

type Error struct {
	ExitCode int
	Err      error
}

func (e *Error) Error() string { return e.Err.Error() }
