package variants

type Ui interface {
	output(msg string)

	// Output to ErrorWriter
	Error(err error)
	// Format and output to Writer
	Printf(format string, args ...interface{})
	// Format success
	Successf(format string, args ...interface{}) string
	// Format warning
	Warnf(format string, args ...interface{}) error
	// Format error
	Errorf(format string, args ...interface{}) error
}
