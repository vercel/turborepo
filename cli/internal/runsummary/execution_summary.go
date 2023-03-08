package runsummary

import "time"

// TaskExecutionSummary contains data about the state of a single task in a turbo run.
// Some fields are updated over time as the task prepares to execute and finishes execution.
type TaskExecutionSummary struct {
	StartAt time.Time `json:"start"`

	Duration time.Duration `json:"duration"`

	// Target which has just changed
	Label string `json:"-"`

	// Its current status
	Status string `json:"status"`

	// Error, only populated for failure statuses
	Err error `json:"error"`
}
