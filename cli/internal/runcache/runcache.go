package runcache

import (
	"bufio"
	"context"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"

	"github.com/fatih/color"
	"github.com/hashicorp/go-hclog"
	"github.com/mitchellh/cli"
	"github.com/spf13/pflag"
	"github.com/vercel/turborepo/cli/internal/cache"
	"github.com/vercel/turborepo/cli/internal/colorcache"
	"github.com/vercel/turborepo/cli/internal/globby"
	"github.com/vercel/turborepo/cli/internal/nodes"
	"github.com/vercel/turborepo/cli/internal/turbopath"
	"github.com/vercel/turborepo/cli/internal/ui"
	"github.com/vercel/turborepo/cli/internal/util"
)

// LogReplayer is a function that is responsible for replaying the contents of a given log file
type LogReplayer = func(logger hclog.Logger, output cli.Ui, logFile turbopath.AbsoluteSystemPath)

// Opts holds the configurable options for a RunCache instance
type Opts struct {
	SkipReads              bool
	SkipWrites             bool
	TaskOutputModeOverride *util.TaskOutputMode
	LogReplayer            LogReplayer
	OutputWatcher          OutputWatcher
}

// AddFlags adds the flags relevant to the runcache package to the given FlagSet
func AddFlags(opts *Opts, flags *pflag.FlagSet) {
	flags.BoolVar(&opts.SkipReads, "force", false, "Ignore the existing cache (to force execution).")
	flags.BoolVar(&opts.SkipWrites, "no-cache", false, "Avoid saving task results to the cache. Useful for development/watch tasks.")

	defaultTaskOutputMode, err := util.ToTaskOutputModeString(util.FullTaskOutput)
	if err != nil {
		panic(err)
	}

	flags.AddFlag(&pflag.Flag{
		Name: "output-logs",
		Usage: `Set type of process output logging. Use "full" to show
all output. Use "hash-only" to show only turbo-computed
task hashes. Use "new-only" to show only new output with
only hashes for cached tasks. Use "none" to hide process
output.`,
		DefValue: defaultTaskOutputMode,
		Value:    &taskOutputModeValue{opts: opts},
	})
	_ = flags.Bool("stream", true, "Unused")
	if err := flags.MarkDeprecated("stream", "[WARNING] The --stream flag is unnecessary and has been deprecated. It will be removed in future versions of turbo."); err != nil {
		// fail fast if we've misconfigured our flags
		panic(err)
	}
}

type taskOutputModeValue struct {
	opts *Opts
}

func (l *taskOutputModeValue) String() string {
	var outputMode util.TaskOutputMode
	if l.opts.TaskOutputModeOverride != nil {
		outputMode = *l.opts.TaskOutputModeOverride
	}
	taskOutputMode, err := util.ToTaskOutputModeString(outputMode)
	if err != nil {
		panic(err)
	}
	return taskOutputMode
}

func (l *taskOutputModeValue) Set(value string) error {
	outputMode, err := util.FromTaskOutputModeString(value)
	if err != nil {
		return fmt.Errorf("must be one of \"%v\"", l.Type())
	}
	l.opts.TaskOutputModeOverride = &outputMode
	return nil
}

func (l *taskOutputModeValue) Type() string {
	var builder strings.Builder

	first := true
	for _, mode := range util.TaskOutputModeStrings {
		if !first {
			builder.WriteString("|")
		}
		first = false
		builder.WriteString(string(mode))
	}
	return builder.String()
}

var _ pflag.Value = &taskOutputModeValue{}

// RunCache represents the interface to the cache for a single `turbo run`
type RunCache struct {
	taskOutputModeOverride *util.TaskOutputMode
	cache                  cache.Cache
	readsDisabled          bool
	writesDisabled         bool
	repoRoot               turbopath.AbsoluteSystemPath
	logReplayer            LogReplayer
	outputWatcher          OutputWatcher
	colorCache             *colorcache.ColorCache
}

// New returns a new instance of RunCache, wrapping the given cache
func New(cache cache.Cache, repoRoot turbopath.AbsoluteSystemPath, opts Opts, colorCache *colorcache.ColorCache) *RunCache {
	rc := &RunCache{
		taskOutputModeOverride: opts.TaskOutputModeOverride,
		cache:                  cache,
		readsDisabled:          opts.SkipReads,
		writesDisabled:         opts.SkipWrites,
		repoRoot:               repoRoot,
		logReplayer:            opts.LogReplayer,
		outputWatcher:          opts.OutputWatcher,
		colorCache:             colorCache,
	}
	if rc.logReplayer == nil {
		rc.logReplayer = defaultLogReplayer
	}
	if rc.outputWatcher == nil {
		rc.outputWatcher = &NoOpOutputWatcher{}
	}
	return rc
}

// TaskCache represents a single task's (package-task?) interface to the RunCache
// and controls access to the task's outputs
type TaskCache struct {
	rc                *RunCache
	repoRelativeGlobs []string
	hash              string
	pt                *nodes.PackageTask
	taskOutputMode    util.TaskOutputMode
	cachingDisabled   bool
	LogFileName       turbopath.AbsoluteSystemPath
}

// RestoreOutputs attempts to restore output for the corresponding task from the cache. Returns true
// if successful.
func (tc TaskCache) RestoreOutputs(ctx context.Context, terminal *cli.PrefixedUi, logger hclog.Logger) (bool, error) {
	if tc.cachingDisabled || tc.rc.readsDisabled {
		if tc.taskOutputMode != util.NoTaskOutput {
			terminal.Output(fmt.Sprintf("cache bypass, force executing %s", ui.Dim(tc.hash)))
		}
		return false, nil
	}
	changedOutputGlobs, err := tc.rc.outputWatcher.GetChangedOutputs(ctx, tc.hash, tc.repoRelativeGlobs)
	if err != nil {
		logger.Warn(fmt.Sprintf("Failed to check if we can skip restoring outputs for %v: %v. Proceeding to check cache", tc.pt.TaskID, err))
		terminal.Warn(ui.Dim(fmt.Sprintf("Failed to check if we can skip restoring outputs for %v: %v. Proceeding to check cache", tc.pt.TaskID, err)))
		changedOutputGlobs = tc.repoRelativeGlobs
	}
	hasChangedOutputs := len(changedOutputGlobs) > 0
	if hasChangedOutputs {
		// Note that we currently don't use the output globs when restoring, but we could in the
		// future to avoid doing unnecessary file I/O
		hit, _, _, err := tc.rc.cache.Fetch(tc.rc.repoRoot.ToString(), tc.hash, changedOutputGlobs)
		if err != nil {
			return false, err
		} else if !hit {
			if tc.taskOutputMode != util.NoTaskOutput {
				terminal.Output(fmt.Sprintf("cache miss, executing %s", ui.Dim(tc.hash)))
			}
			return false, nil
		}
		if err := tc.rc.outputWatcher.NotifyOutputsWritten(ctx, tc.hash, tc.repoRelativeGlobs); err != nil {
			// Don't fail the whole operation just because we failed to watch the outputs
			logger.Warn(fmt.Sprintf("Failed to mark outputs as cached for %v: %v", tc.pt.TaskID, err))
			terminal.Warn(ui.Dim(fmt.Sprintf("Failed to mark outputs as cached for %v: %v", tc.pt.TaskID, err)))
		}
	} else {
		logger.Debug(fmt.Sprintf("Skipping cache check for %v, outputs have not changed since previous run.", tc.pt.TaskID))
	}

	switch tc.taskOutputMode {
	// When only showing new task output, cached output should only show the computed hash
	case util.NewTaskOutput:
		fallthrough
	case util.HashTaskOutput:
		terminal.Output(fmt.Sprintf("cache hit, suppressing output %s", ui.Dim(tc.hash)))
	case util.FullTaskOutput:
		logger.Debug("log file", "path", tc.LogFileName)
		if tc.LogFileName.FileExists() {
			// The task label is baked into the log file, so we need to grab the underlying Ui
			// instance in order to not duplicate it
			tc.rc.logReplayer(logger, terminal.Ui, tc.LogFileName)
		}
	default:
		// NoLogs, do not output anything
	}
	return true, nil
}

// nopWriteCloser is modeled after io.NopCloser, which is for Readers
type nopWriteCloser struct {
	io.Writer
}

func (nopWriteCloser) Close() error { return nil }

type fileWriterCloser struct {
	io.Writer
	file  *os.File
	bufio *bufio.Writer
}

func (fwc *fileWriterCloser) Close() error {
	if err := fwc.bufio.Flush(); err != nil {
		return err
	}
	return fwc.file.Close()
}

// OutputWriter creates a sink suitable for handling the output of the command associated
// with this task.
func (tc TaskCache) OutputWriter() (io.WriteCloser, error) {
	if tc.cachingDisabled || tc.rc.writesDisabled {
		return nopWriteCloser{os.Stdout}, nil
	}
	// Setup log file
	if err := tc.LogFileName.EnsureDir(); err != nil {
		return nil, err
	}
	output, err := tc.LogFileName.Create()
	if err != nil {
		return nil, err
	}
	colorPrefixer := tc.rc.colorCache.PrefixColor(tc.pt.PackageName)
	prettyTaskPrefix := colorPrefixer(tc.pt.OutputPrefix())
	bufWriter := bufio.NewWriter(output)
	if _, err := bufWriter.WriteString(fmt.Sprintf("%s: cache hit, replaying output %s\n", prettyTaskPrefix, ui.Dim(tc.hash))); err != nil {
		// We've already errored, we don't care if there's a further error closing the file we just
		// failed to write to.
		_ = output.Close()
		return nil, err
	}
	fwc := &fileWriterCloser{
		file:  output,
		bufio: bufWriter,
	}
	if tc.taskOutputMode == util.NoTaskOutput || tc.taskOutputMode == util.HashTaskOutput {
		// only write to log file, not to stdout
		fwc.Writer = bufWriter
	} else {
		fwc.Writer = io.MultiWriter(os.Stdout, bufWriter)
	}
	return fwc, nil
}

var _emptyIgnore []string

// SaveOutputs is responsible for saving the outputs of task to the cache, after the task has completed
func (tc TaskCache) SaveOutputs(ctx context.Context, logger hclog.Logger, terminal cli.Ui, duration int) error {
	if tc.cachingDisabled || tc.rc.writesDisabled {
		return nil
	}

	logger.Debug("caching output", "outputs", tc.repoRelativeGlobs)

	filesToBeCached, err := globby.GlobFiles(tc.rc.repoRoot.ToStringDuringMigration(), tc.repoRelativeGlobs, _emptyIgnore)
	if err != nil {
		return err
	}

	relativePaths := make([]string, len(filesToBeCached))

	for index, value := range filesToBeCached {
		relativePath, err := tc.rc.repoRoot.RelativePathString(value)
		if err != nil {
			logger.Error("error", err)
			terminal.Error(fmt.Sprintf("%s%s", ui.ERROR_PREFIX, color.RedString(" %v", fmt.Errorf("File path cannot be made relative: %w", err))))
			continue
		}
		relativePaths[index] = relativePath
	}

	if err = tc.rc.cache.Put(tc.pt.Pkg.Dir.ToStringDuringMigration(), tc.hash, duration, relativePaths); err != nil {
		return err
	}
	err = tc.rc.outputWatcher.NotifyOutputsWritten(ctx, tc.hash, tc.repoRelativeGlobs)
	if err != nil {
		// Don't fail the cache write because we also failed to record it, we will just do
		// extra I/O in the future restoring files that haven't changed from cache
		logger.Warn(fmt.Sprintf("Failed to mark outputs as cached for %v: %v", tc.pt.TaskID, err))
		terminal.Warn(ui.Dim(fmt.Sprintf("Failed to mark outputs as cached for %v: %v", tc.pt.TaskID, err)))
	}
	return nil
}

// TaskCache returns a TaskCache instance, providing an interface to the underlying cache specific
// to this run and the given PackageTask
func (rc *RunCache) TaskCache(pt *nodes.PackageTask, hash string) TaskCache {
	logFileName := rc.repoRoot.UnsafeJoin(pt.RepoRelativeLogFile())
	hashableOutputs := pt.HashableOutputs()
	repoRelativeGlobs := make([]string, len(hashableOutputs))
	for index, output := range hashableOutputs {
		repoRelativeGlobs[index] = filepath.Join(pt.Pkg.Dir.ToStringDuringMigration(), output)
	}

	taskOutputMode := pt.TaskDefinition.OutputMode
	if rc.taskOutputModeOverride != nil {
		taskOutputMode = *rc.taskOutputModeOverride
	}

	return TaskCache{
		rc:                rc,
		repoRelativeGlobs: repoRelativeGlobs,
		hash:              hash,
		pt:                pt,
		taskOutputMode:    taskOutputMode,
		cachingDisabled:   !pt.TaskDefinition.ShouldCache,
		LogFileName:       logFileName,
	}
}

// defaultLogReplayer will try to replay logs back to the given Ui instance
func defaultLogReplayer(logger hclog.Logger, output cli.Ui, logFileName turbopath.AbsoluteSystemPath) {
	logger.Debug("start replaying logs")
	f, err := logFileName.Open()
	if err != nil {
		output.Warn(fmt.Sprintf("error reading logs: %v", err))
		logger.Error(fmt.Sprintf("error reading logs: %v", err.Error()))
	}
	defer func() { _ = f.Close() }()
	scan := bufio.NewScanner(f)
	for scan.Scan() {
		output.Output(string(scan.Bytes())) //Writing to Stdout
	}
	logger.Debug("finish replaying logs")
}
