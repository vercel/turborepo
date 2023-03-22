package runsummary

type vercelRunResponse struct {
	ID string
}

type vercelRunPayload struct {
	// ID is set by the backend, including it here for completeness, but we never fill this in.
	ID string `json:"vercelId,omitempty"`

	// StartTime is when this run was started
	StartTime int `json:"startTime,omitempty"`

	// EndTime is when this run ended. We will never be submitting start and endtime at the same time.
	EndTime int `json:"endTime,omitempty"`

	// Status is
	Status string `json:"status,omitempty"`

	// Type should be hardcoded to TURBO
	Type string `json:"type,omitempty"`

	// TODO: we need to add these in
	// originationUser string
	// gitBranch       string
	// gitSha          string
	// context         string
	// command         string
}

func newVercelRunCreatePayload(runsummary *RunSummary) *vercelRunPayload {
	startTime := int(runsummary.ExecutionSummary.startedAt.UnixMilli())
	return &vercelRunPayload{
		StartTime: startTime,
		Status:    "started",
		Type:      "TURBO",
	}
}

func newVercelDonePayload() *vercelRunPayload {
	// TODO: add in the endTime here.
	return &vercelRunPayload{
		Status: "completed",
	}
}
