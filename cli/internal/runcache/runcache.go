package runcache

import (
	"bufio"
	"fmt"
	"io"
	"os"
	"path/filepath"

	"github.com/fatih/color"
	"github.com/hashicorp/go-hclog"
	"github.com/mitchellh/cli"
	"github.com/spf13/pflag"
	"github.com/vercel/turborepo/cli/internal/cache"
	"github.com/vercel/turborepo/cli/internal/fs"
	"github.com/vercel/turborepo/cli/internal/globby"
	"github.com/vercel/turborepo/cli/internal/nodes"
	"github.com/vercel/turborepo/cli/internal/ui"
)

// LogsMode defines the ways turbo can output logs during a run for cached and not cached tasks
type LogsMode int

// LogReplayer is a function that is responsible for replaying the contents of a given log file
type LogReplayer = func(logger hclog.Logger, output cli.Ui, logFile fs.AbsolutePath)

const (
	// FullLogs - show all,
	FullLogs LogsMode = iota
	// HashLogs - only show task hash
	HashLogs
	// NoLogs - show nothing
	NoLogs
)

// Opts holds the configurable options for a RunCache instance
type Opts struct {
	SkipReads         bool
	SkipWrites        bool
	CacheHitLogsMode  LogsMode
	CacheMissLogsMode LogsMode
	LogReplayer       LogReplayer
}

// AddFlags adds the flags relevant to the runcache package to the given FlagSet
func AddFlags(opts *Opts, flags *pflag.FlagSet) {
	flags.BoolVar(&opts.SkipReads, "force", false, "Ignore the existing cache (to force execution).")
	flags.BoolVar(&opts.SkipWrites, "no-cache", false, "Avoid saving task results to the cache. Useful for development/watch tasks.")
	flags.AddFlag(&pflag.Flag{
		Name:     "output-logs",
		Usage:    _outputModeHelp,
		DefValue: "full",
		Value:    &logsModeValue{opts: opts},
	})
	_ = flags.Bool("stream", true, "Unused")
	if err := flags.MarkDeprecated("stream", "[WARNING] The --stream flag is unnecesary and has been deprecated. It will be removed in future versions of turbo."); err != nil {
		// fail fast if we've misconfigured our flags
		panic(err)
	}
}

var _outputModeHelp = `Set type of process output logging. Use full to show
all output. Use hash-only to show only turbo-computed
task hashes. Use new-only to show only new output with
only hashes for cached tasks. Use none to hide process
output.`

type logsModeValue struct {
	opts *Opts
}

func (l *logsModeValue) String() string {
	if l.opts.CacheHitLogsMode == FullLogs && l.opts.CacheMissLogsMode == FullLogs {
		return "full"
	} else if l.opts.CacheHitLogsMode == NoLogs && l.opts.CacheMissLogsMode == NoLogs {
		return "none"
	} else if l.opts.CacheHitLogsMode == HashLogs && l.opts.CacheMissLogsMode == HashLogs {
		return "hash-only"
	} else if l.opts.CacheHitLogsMode == HashLogs && l.opts.CacheMissLogsMode == FullLogs {
		return "new-only"
	} else {
		panic(fmt.Sprintf("Invalid output logs mode. Hit %v, miss %v", l.opts.CacheHitLogsMode, l.opts.CacheMissLogsMode))
	}
}

func (l *logsModeValue) Set(value string) error {
	switch value {
	case "full":
		l.opts.CacheMissLogsMode = FullLogs
		l.opts.CacheHitLogsMode = FullLogs
	case "none":
		l.opts.CacheMissLogsMode = NoLogs
		l.opts.CacheHitLogsMode = NoLogs
	case "hash-only":
		l.opts.CacheMissLogsMode = HashLogs
		l.opts.CacheHitLogsMode = HashLogs
	case "new-only":
		l.opts.CacheMissLogsMode = FullLogs
		l.opts.CacheHitLogsMode = HashLogs
	default:
		return fmt.Errorf("unknown output-mode: %v", value)
	}
	return nil
}

func (l *logsModeValue) Type() string {
	return "full|none|hash-only|new-only"
}

var _ pflag.Value = &logsModeValue{}

// RunCache represents the interface to the cache for a single `turbo run`
type RunCache struct {
	cacheHitLogsMode  LogsMode
	cacheMissLogsMode LogsMode
	cache             cache.Cache
	readsDisabled     bool
	writesDisabled    bool
	repoRoot          fs.AbsolutePath
	logReplayer       LogReplayer
}

// New returns a new instance of RunCache, wrapping the given cache
func New(cache cache.Cache, repoRoot fs.AbsolutePath, opts Opts) *RunCache {
	rc := &RunCache{
		cacheHitLogsMode:  opts.CacheHitLogsMode,
		cacheMissLogsMode: opts.CacheMissLogsMode,
		cache:             cache,
		readsDisabled:     opts.SkipReads,
		writesDisabled:    opts.SkipWrites,
		repoRoot:          repoRoot,
		logReplayer:       opts.LogReplayer,
	}
	if rc.logReplayer == nil {
		rc.logReplayer = defaultLogReplayer
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
	cachingDisabled   bool
	LogFileName       fs.AbsolutePath
}

// RestoreOutputs attempts to restore output for the corresponding task from the cache. Returns true
// if successful.
func (tc TaskCache) RestoreOutputs(terminal *cli.PrefixedUi, logger hclog.Logger) (bool, error) {
	if tc.cachingDisabled || tc.rc.readsDisabled {
		if tc.rc.cacheHitLogsMode != NoLogs {
			terminal.Output(fmt.Sprintf("cache bypass, force executing %s", ui.Dim(tc.hash)))
		}
		return false, nil
	}
	// TODO(gsoltis): check if we need to restore goes here
	// That will be an opportunity to prune down the set of outputs as well
	hit, _, _, err := tc.rc.cache.Fetch(tc.rc.repoRoot.ToString(), tc.hash, tc.repoRelativeGlobs)
	if err != nil {
		return false, err
	} else if !hit {
		if tc.rc.cacheMissLogsMode != NoLogs {
			terminal.Output(fmt.Sprintf("cache miss, executing %s", ui.Dim(tc.hash)))
		}
		return false, nil
	}
	switch tc.rc.cacheHitLogsMode {
	case HashLogs:
		terminal.Output(fmt.Sprintf("cache hit, suppressing output %s", ui.Dim(tc.hash)))
	case FullLogs:
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
	bufWriter := bufio.NewWriter(output)
	if _, err := bufWriter.WriteString(fmt.Sprintf("%s: cache hit, replaying output %s\n", tc.pt.OutputPrefix(), ui.Dim(tc.hash))); err != nil {
		// We've already errored, we don't care if there's a further error closing the file we just
		// failed to write to.
		_ = output.Close()
		return nil, err
	}
	fwc := &fileWriterCloser{
		file:  output,
		bufio: bufWriter,
	}
	if tc.rc.cacheMissLogsMode == NoLogs || tc.rc.cacheMissLogsMode == HashLogs {
		// only write to log file, not to stdout
		fwc.Writer = bufWriter
	} else {
		fwc.Writer = io.MultiWriter(os.Stdout, bufWriter)
	}
	return fwc, nil
}

var _emptyIgnore []string

// SaveOutputs is responsible for saving the outputs of task to the cache, after the task has completed
func (tc TaskCache) SaveOutputs(logger hclog.Logger, terminal cli.Ui, duration int) error {
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

	return tc.rc.cache.Put(tc.pt.Pkg.Dir, tc.hash, duration, relativePaths)
}

// TaskCache returns a TaskCache instance, providing an interface to the underlying cache specific
// to this run and the given PackageTask
func (rc *RunCache) TaskCache(pt *nodes.PackageTask, hash string) TaskCache {
	logFileName := rc.repoRoot.Join(pt.RepoRelativeLogFile())
	hashableOutputs := pt.HashableOutputs()
	repoRelativeGlobs := make([]string, len(hashableOutputs))
	for index, output := range hashableOutputs {
		repoRelativeGlobs[index] = filepath.Join(pt.Pkg.Dir, output)
	}
	return TaskCache{
		rc:                rc,
		repoRelativeGlobs: repoRelativeGlobs,
		hash:              hash,
		pt:                pt,
		cachingDisabled:   !pt.TaskDefinition.ShouldCache,
		LogFileName:       logFileName,
	}
}

// defaultLogReplayer will try to replay logs back to the given Ui instance
func defaultLogReplayer(logger hclog.Logger, output cli.Ui, logFileName fs.AbsolutePath) {
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
