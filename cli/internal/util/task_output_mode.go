package util

import (
	"encoding/json"
	"fmt"
)

// TaskOutputMode defines the ways turbo can display task output during a run
type TaskOutputMode int

const (
	// FullTaskOutput will show all task output
	FullTaskOutput TaskOutputMode = iota
	// NoTaskOutput will hide all task output
	NoTaskOutput
	// HashTaskOutput will display turbo-computed task hashes
	HashTaskOutput
	// NewTaskOutput will show all new task output and turbo-computed task hashes for cached output
	NewTaskOutput
	// ErrorTaskOutput will show task output for failures only; no cache miss/hit messages are emitted
	ErrorTaskOutput
)

const (
	fullTaskOutputString  = "full"
	noTaskOutputString    = "none"
	hashTaskOutputString  = "hash-only"
	newTaskOutputString   = "new-only"
	errorTaskOutputString = "errors-only"
)

// TaskOutputModeStrings is an array containing the string representations for task output modes
var TaskOutputModeStrings = []string{
	fullTaskOutputString,
	noTaskOutputString,
	hashTaskOutputString,
	newTaskOutputString,
	errorTaskOutputString,
}

// FromTaskOutputModeString converts a task output mode's string representation into the enum value
func FromTaskOutputModeString(value string) (TaskOutputMode, error) {
	switch value {
	case fullTaskOutputString:
		return FullTaskOutput, nil
	case noTaskOutputString:
		return NoTaskOutput, nil
	case hashTaskOutputString:
		return HashTaskOutput, nil
	case newTaskOutputString:
		return NewTaskOutput, nil
	case errorTaskOutputString:
		return ErrorTaskOutput, nil
	}

	return FullTaskOutput, fmt.Errorf("invalid task output mode: %v", value)
}

// ToTaskOutputModeString converts a task output mode enum value into the string representation
func ToTaskOutputModeString(value TaskOutputMode) (string, error) {
	switch value {
	case FullTaskOutput:
		return fullTaskOutputString, nil
	case NoTaskOutput:
		return noTaskOutputString, nil
	case HashTaskOutput:
		return hashTaskOutputString, nil
	case NewTaskOutput:
		return newTaskOutputString, nil
	case ErrorTaskOutput:
		return errorTaskOutputString, nil
	}

	return "", fmt.Errorf("invalid task output mode: %v", value)
}

// UnmarshalJSON converts a task output mode string representation into an enum
func (c *TaskOutputMode) UnmarshalJSON(data []byte) error {
	var rawTaskOutputMode string
	if err := json.Unmarshal(data, &rawTaskOutputMode); err != nil {
		return err
	}

	taskOutputMode, err := FromTaskOutputModeString(rawTaskOutputMode)
	if err != nil {
		return err
	}

	*c = taskOutputMode
	return nil
}

// MarshalJSON converts a task output mode to its string representation
func (c TaskOutputMode) MarshalJSON() ([]byte, error) {
	outputModeString, err := ToTaskOutputModeString(c)
	if err != nil {
		return nil, err
	}
	return json.Marshal(outputModeString)
}
