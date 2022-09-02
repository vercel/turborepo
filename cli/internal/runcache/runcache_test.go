package runcache

import (
	"io"
	"os"
	"testing"

	"github.com/vercel/turborepo/cli/internal/colorcache"
	"github.com/vercel/turborepo/cli/internal/fs"
	"github.com/vercel/turborepo/cli/internal/nodes"
	"github.com/vercel/turborepo/cli/internal/util"
	"gotest.tools/v3/assert"
)

func Test_OutputWriter(t *testing.T) {
	// Setup
	savedOutput := os.Stdout
	tempFile := fs.AbsolutePath(t.TempDir() + "temp.log")
	taskCache := TaskCache{
		rc: &RunCache{
			writesDisabled: false,
			colorCache:     colorcache.New(),
			prefixStripped: false,
		},
		pt: &nodes.PackageTask{
			PackageName: "testPackageName",
		},
		cachingDisabled: false,
		LogFileName:     tempFile,
	}
	someLogs := "First line of log.\nSecond line.\n\tThird line a little different"
	expectedFileLogs := `cache hit, replaying output 
First line of log.
Second line.
	Third line a little different`

	t.Run("With active prefix", func(t *testing.T) {
		readOutputPipe, writeOutputPipe, _ := os.Pipe()
		os.Stdout = writeOutputPipe
		// Given
		taskCache.rc.prefixStripped = false

		expectedOutputLogs := `testPackageName:: First line of log.
testPackageName:: Second line.
testPackageName:: 	Third line a little different`

		// When
		writer, err := taskCache.OutputWriter()
		if err != nil {
			t.Error("Error during Output Writer:", err)
		}
		if _, err := writer.Write([]byte(someLogs)); err != nil {
			t.Error("Error when writing logs:\n", err)
		}
		if err := writer.Close(); err != nil {
			t.Error("Error when closing Writer:\n", err)
		}
		writeOutputPipe.Close()

		// Then
		outputContent, _ := io.ReadAll(readOutputPipe)
		assert.Equal(t, expectedOutputLogs, string(outputContent), "Output log content is different from expected")
		fileContent, _ := os.ReadFile(tempFile.ToString())
		assert.Equal(t, expectedFileLogs, string(fileContent), "File log content is different from expected")
	})

	t.Run("Without active prefix", func(t *testing.T) {
		readOutputPipe, writeOutputPipe, _ := os.Pipe()
		os.Stdout = writeOutputPipe
		// Given
		taskCache.rc.prefixStripped = true
		expectedOutputLogs := `First line of log.
Second line.
	Third line a little different`

		// When
		writer, err := taskCache.OutputWriter()
		if err != nil {
			t.Error("Error during Output Writer:", err)
		}
		if _, err := writer.Write([]byte(someLogs)); err != nil {
			t.Error("Error when writing logs:\n", err)
		}
		if err := writer.Close(); err != nil {
			t.Error("Error when closing Writer:\n", err)
		}
		writeOutputPipe.Close()

		// Then
		outputContent, _ := io.ReadAll(readOutputPipe)
		assert.Equal(t, expectedOutputLogs, string(outputContent), "Output log content is different from expected")
		fileContent, _ := os.ReadFile(tempFile.ToString())
		assert.Equal(t, expectedFileLogs, string(fileContent), "File log content is different from expected")
	})

	t.Run("With taskOutputMode set to util.NoTaskOutput", func(t *testing.T) {
		readOutputPipe, writeOutputPipe, _ := os.Pipe()
		os.Stdout = writeOutputPipe
		// Given
		taskCache.taskOutputMode = util.NoTaskOutput
		expectedOutputLogs := ""

		// When
		writer, err := taskCache.OutputWriter()
		if err != nil {
			t.Error("Error during Output Writer:", err)
		}
		if _, err := writer.Write([]byte(someLogs)); err != nil {
			t.Error("Error when writing logs:\n", err)
		}
		if err := writer.Close(); err != nil {
			t.Error("Error when closing Writer:\n", err)
		}
		writeOutputPipe.Close()

		// Then
		outputContent, _ := io.ReadAll(readOutputPipe)
		assert.Equal(t, expectedOutputLogs, string(outputContent), "Output log content is different from expected")
		fileContent, _ := os.ReadFile(tempFile.ToString())
		assert.Equal(t, expectedFileLogs, string(fileContent), "File log content is different from expected")
	})

	t.Cleanup(func() {
		os.Stdout = savedOutput
	})
}
