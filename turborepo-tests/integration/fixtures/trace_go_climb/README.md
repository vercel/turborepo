# trace-go-climb

A Go service (`services/billing`) whose tests read files of the repository through
a helper that takes the path (`config.Load`), with relative paths that climb
out of the test's package directory, and two apps with files of the same names.
