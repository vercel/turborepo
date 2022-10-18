package turbostate

type RepoState struct {
	Root string `json:"root"`
	Mode string `json:"mode"`
}

type Command struct {
	Id      string                 `json:"id"`
	Payload map[string]interface{} `json:"payload"`
}

type Args struct {
	Api        *string  `json:"api"`
	Color      bool     `json:"color"`
	Cpuprofile *string  `json:"cpuprofile"`
	Cwd        *string  `json:"cwd"`
	Heap       *string  `json:"heap"`
	Login      *string  `json:"login"`
	NoColor    bool     `json:"noColor"`
	Preflight  bool     `json:"preflight"`
	Team       *string  `json:"team"`
	Token      *string  `json:"token"`
	Trace      *string  `json:"trace"`
	Verbosity  *uint8   `json:"verbosity"`
	Command    *Command `json:"command"`
}

type TurboState struct {
	RepoState  RepoState `json:"repo_state"`
	ParsedArgs Args      `json:"parsed_args"`
	RawArgs    []string  `json:"raw_args"`
}
