package util

// TaskOutputMode defines the ways turbo can display task output during a run
type TaskOutputMode string

const (
	// FullTaskOutput will show all task output
	FullTaskOutput TaskOutputMode = "full"
	// NoTaskOutput will hide all task output
	NoTaskOutput TaskOutputMode = "none"
	// HashTaskOutput will display turbo-computed task hashes
	HashTaskOutput TaskOutputMode = "hash-only"
	// NewTaskOutput will show all new task output and turbo-computed task hashes for cached output
	NewTaskOutput TaskOutputMode = "new-only"
)

// TaskOutputModes contains all of the valid task output modes
var TaskOutputModes = []TaskOutputMode{
	FullTaskOutput,
	NoTaskOutput,
	HashTaskOutput,
	NewTaskOutput,
}

// IsValidTaskOutputMode returns whether or not a value is a valid task output mode
func IsValidTaskOutputMode(value string) bool {
	for _, mode := range TaskOutputModes {
		if string(mode) == value {
			return true
		}
	}
	return false
}
