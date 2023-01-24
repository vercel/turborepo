package chrometracing

// Close overwrites the trailing (,\n) with (]\n) and closes the trace file.
// Close is implemented in a separate file to keep a separation between custom
// code and upstream from github.com/google/chrometracing. Additionally, we can
// enable linting for code we author, while leaving upstream code alone.
func Close() error {
	trace.fileMu.Lock()
	defer trace.fileMu.Unlock()
	// Seek backwards two bytes (,\n)
	if _, err := trace.file.Seek(-2, 1); err != nil {
		return err
	}
	// Write 1 byte, ']', leaving the trailing '\n' in place
	if _, err := trace.file.Write([]byte{']'}); err != nil {
		return err
	}
	// Force the filesystem to write to disk
	if err := trace.file.Sync(); err != nil {
		return err
	}
	if err := trace.file.Close(); err != nil {
		return err
	}
	return nil
}
