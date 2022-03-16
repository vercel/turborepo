package run

import (
	"bufio"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"time"

	"github.com/vercel/turborepo/cli/internal/cache"
	"github.com/vercel/turborepo/cli/internal/config"
	"github.com/vercel/turborepo/cli/internal/fs"
	"github.com/vercel/turborepo/cli/internal/globby"
	"github.com/vercel/turborepo/cli/internal/ui"
	"github.com/vercel/turborepo/cli/internal/util"

	"github.com/fatih/color"
	"github.com/hashicorp/go-hclog"
	"github.com/mitchellh/cli"
	"github.com/pkg/errors"
)

// LogsCommand is a Command implementation that allows the user to view log replays
type LogsCommand struct {
	Config *config.Config
	Ui     *cli.ColoredUi
}

// hashMetadata represents the files and duration metadata associated with
// a hash of a task
type hashMetadata struct {
	Hash        string
	ReplayPaths []string
	Duration    int
	Datetime    time.Time
}

type SortMode string

const (
	TaskSort     SortMode = "task"
	DurationSort SortMode = "duration"
	AlnumSort    SortMode = "alnum"
	DatetimeSort SortMode = "datetime"
	QuerySort    SortMode = "query"
	NothingSort  SortMode = "n/a"
)

// Synopsis of run command
func (c *LogsCommand) Synopsis() string {
	return "Review the most recently run tasks logs"
}

// Help returns information about the `run` command
func (c *LogsCommand) Help() string {
	helpText := `
Usage: turbo logs [--list] [<hashes>]

    Review the most recently run tasks logs.

Options:
  --help                 Show this message.
  --list                 List the hashes available for viewing from the cache.
  --list-format          Specify output format for --list. Use hash to only
                         show the hashes. Use relevant to include metadata
                         relevant to sorting if any. (default hash)
  --all                  Show all results, not just results from the most
                         recent run command execution.
  --sort                 Set mode to order results. Use task to order by the
                         previous run's execution order. Use duration to order
                         by time taken (lowest to highest). Use datetime to
                         order by timestamp of the log file. Use alnum to order
                         alphanumerically. Normally defaults to task. If
                         specific hashes are given, defaults to their order.
                         If --all, defaults to datetime. (default task)
  --reverse              Reverse order while sorting.
  --output-logs          Set type of replay output logs. Use full to show all
                         output. Use hash-only to show only the turbo-computed
                         task hash lines. (default full)
  --cache-dir            Specify local filesystem cache directory.
                         (default "./node_modules/.cache/turbo")
  --last-run-path        Specify path to last run file to load.
                         (default <--cache-dir>/last-run.log)
`
	return strings.TrimSpace(helpText)
}

// Run finds and replays task logs in the monorepo
func (c *LogsCommand) Run(args []string) int {
	logsOptions, err := parseLogsArgs(args, c.Ui)
	if err != nil {
		c.logError(c.Config.Logger, "", err)
		return 1
	}

	c.Config.Logger.Trace("lastRunPath", "value", logsOptions.lastRunPath)
	c.Config.Logger.Trace("list", "value", logsOptions.listOnly)
	c.Config.Logger.Trace("sort", "value", logsOptions.sortType)
	c.Config.Logger.Trace("reverse", "value", logsOptions.reverseSort)

	var lastRunHashes []string
	if logsOptions.sortType == TaskSort || !logsOptions.includeAll {
		if !fs.FileExists(logsOptions.lastRunPath) {
			c.logError(c.Config.Logger, "", fmt.Errorf("failed to resolve last run file: %v", logsOptions.lastRunPath))
			metadataPaths := globby.GlobFiles(logsOptions.cacheFolder, []string{"*-meta.json"}, []string{})
			if len(metadataPaths) > 0 {
				c.logInfo(c.Config.Logger, "other logs found, use --all to view them")
			}
			return 1
		}
		lastRunHashes, err = cache.ReadHashesFile(logsOptions.lastRunPath)
		if err != nil {
			c.logError(c.Config.Logger, "", err)
			return 1
		}
	}
	c.Config.Logger.Trace("lastRunHashes", "value", lastRunHashes)

	specificHashes := lastRunHashes
	if len(logsOptions.queryHashes) > 0 {
		specificHashes = logsOptions.queryHashes
	}

	// find and collect cached hashes and durations from metadata json
	// also get file modified datetime from filesystem
	var hashes []hashMetadata
	if len(specificHashes) > 0 {
		for _, hash := range specificHashes {
			replayPaths := globby.GlobFiles(filepath.Join(logsOptions.cacheFolder, hash), []string{"**/.turbo/turbo-*.log"}, []string{})
			metadataPath := filepath.Join(logsOptions.cacheFolder, hash+"-meta.json")
			metadata, err := cache.ReadCacheMetaFile(metadataPath)
			duration := -1
			if err == nil {
				duration = metadata.Duration
			} else {
				c.logWarning(c.Config.Logger, "", fmt.Errorf("cannot read metadata file: %v: %w", metadataPath, err))
			}
			file, err := os.Stat(metadataPath)
			datetime := time.Time{}
			if err == nil {
				datetime = file.ModTime()
			} else {
				c.logWarning(c.Config.Logger, "", fmt.Errorf("cannot get modification time of metadata file: %v: %w", metadataPath, err))
			}
			hashes = append(hashes, hashMetadata{
				Hash:        hash,
				ReplayPaths: replayPaths,
				Duration:    duration,
				Datetime:    datetime,
			})
		}
	} else {
		metadataPaths := globby.GlobFiles(logsOptions.cacheFolder, []string{"*-meta.json"}, []string{})
		for _, metadataPath := range metadataPaths {
			metadata, err := cache.ReadCacheMetaFile(metadataPath)
			if err != nil {
				c.logWarning(c.Config.Logger, "", fmt.Errorf("cannot read metadata file: %v: %w", metadataPath, err))
				continue
			}
			file, err := os.Stat(metadataPath)
			datetime := time.Time{}
			if err == nil {
				datetime = file.ModTime()
			} else {
				c.logWarning(c.Config.Logger, "", fmt.Errorf("cannot get modification time of metadata file: %v: %w", metadataPath, err))
			}
			replayPaths := globby.GlobFiles(filepath.Join(logsOptions.cacheFolder, metadata.Hash, ".turbo"), []string{"turbo-*.log"}, []string{})
			hashes = append(hashes, hashMetadata{
				Hash:        metadata.Hash,
				ReplayPaths: replayPaths,
				Duration:    metadata.Duration,
				Datetime:    datetime,
			})
		}
	}
	c.Config.Logger.Trace("hashes before sort", "value", hashes)

	// sort task list
	cmp := createAlnumComparator(hashes, logsOptions.reverseSort)
	if logsOptions.sortType == DurationSort {
		cmp = createDurationComparator(hashes, logsOptions.reverseSort)
	} else if logsOptions.sortType == TaskSort {
		cmp = createReferenceIndexComparator(hashes, lastRunHashes, logsOptions.reverseSort)
	} else if logsOptions.sortType == QuerySort {
		cmp = createReferenceIndexComparator(hashes, logsOptions.queryHashes, logsOptions.reverseSort)
	} else if logsOptions.sortType == DatetimeSort {
		cmp = createDatetimeComparator(hashes, logsOptions.reverseSort)
	}
	sort.SliceStable(hashes, cmp)

	// output replay logs from sorted task list
	for _, hash := range hashes {
		if logsOptions.listOnly {
			if logsOptions.listType == "relevant" {
				if logsOptions.sortType == DurationSort {
					c.Ui.Output(fmt.Sprintf("%v %v", hash.Hash, hash.Duration))
					continue
				}
			}
			c.Ui.Output(hash.Hash)
			continue
		}
		if len(hash.ReplayPaths) == 0 {
			c.logInfo(c.Config.Logger, fmt.Sprintf("%v: no logs found to replay", hash.Hash))
		}
		for _, replayPath := range hash.ReplayPaths {
			file, err := os.Open(replayPath)
			if err != nil {
				c.logWarning(c.Config.Logger, "", fmt.Errorf("error reading logs: %w", err))
				continue
			}
			defer file.Close()
			scan := bufio.NewScanner(file)
			if logsOptions.outputLogsMode == "hash" {
				scan.Scan()
				c.Ui.Output(ui.StripAnsi(string(scan.Bytes())))
			} else {
				for scan.Scan() {
					c.Ui.Output(ui.StripAnsi(string(scan.Bytes())))
				}
			}
		}
	}

	return 0
}

func createDurationComparator(hashes []hashMetadata, reverse bool) func(int, int) bool {
	if reverse {
		return func(i, j int) bool {
			return hashes[i].Duration > hashes[j].Duration
		}
	}
	return func(i, j int) bool {
		return hashes[i].Duration <= hashes[j].Duration
	}
}

func createDatetimeComparator(hashes []hashMetadata, reverse bool) func(int, int) bool {
	if reverse {
		return func(i, j int) bool {
			return hashes[i].Datetime.After(hashes[j].Datetime)
		}
	}
	return func(i, j int) bool {
		return hashes[i].Datetime.Before(hashes[j].Datetime) || hashes[i].Datetime.Equal(hashes[j].Datetime)
	}
}

func createAlnumComparator(hashes []hashMetadata, reverse bool) func(int, int) bool {
	if reverse {
		return func(i, j int) bool {
			return hashes[i].Hash > hashes[j].Hash
		}
	}
	return func(i, j int) bool {
		return hashes[i].Hash <= hashes[j].Hash
	}
}

func createReferenceIndexComparator(hashes []hashMetadata, refHashes []string, reverse bool) func(int, int) bool {
	hashToIndex := make(map[string]int)
	for i, hash := range refHashes {
		hashToIndex[hash] = i
	}
	if reverse {
		return func(i, j int) bool {
			return hashToIndex[hashes[i].Hash] > hashToIndex[hashes[j].Hash]
		}
	}
	return func(i, j int) bool {
		return hashToIndex[hashes[i].Hash] <= hashToIndex[hashes[j].Hash]
	}
}

// LogsOptions holds the current run operations configuration
type LogsOptions struct {
	// Current working directory
	cwd string
	// Cache folder
	cacheFolder string
	// Only output task hashes
	listOnly bool
	// Adjust output format
	//  hash - only show hash
	//  relevant - include relevant metadata
	listType string
	// Show all results, not only from the last run
	includeAll bool
	// Path to last run file
	lastRunPath string
	// Order by
	//  task - last run's execution order
	//  duration - duration of each task (low to high)
	//  alnum - alphanumerically on hash string
	//  query - match order of queryHashes
	sortType SortMode
	// True to reverse output order
	reverseSort bool
	// List of requested hashes to retrieve
	// in user-provided order
	queryHashes []string
	// Replay task logs output mode
	// full - show all,
	// hash - only show task hash
	// TODO: refactor to use run.LogsMode
	outputLogsMode string
}

func getDefaultLogsOptions() *LogsOptions {
	return &LogsOptions{
		listOnly:       false,
		listType:       "hash",
		includeAll:     false,
		sortType:       "task",
		reverseSort:    false,
		outputLogsMode: "full",
	}
}

func parseLogsArgs(args []string, output cli.Ui) (*LogsOptions, error) {
	var logsOptions = getDefaultLogsOptions()

	cwd, err := os.Getwd()
	if err != nil {
		return nil, fmt.Errorf("invalid working directory: %w", err)
	}
	logsOptions.cwd = cwd

	unresolvedCacheFolder := filepath.FromSlash("./node_modules/.cache/turbo")
	unresolvedLastRunPath := ""
	unresolvedSortType := NothingSort
	queryHashesSet := make(util.Set)

	for _, arg := range args {
		if strings.HasPrefix(arg, "--") {
			switch {
			case strings.HasPrefix(arg, "--cwd="):
				if len(arg[len("--cwd="):]) > 0 {
					logsOptions.cwd = arg[len("--cwd="):]
				} else {
					logsOptions.cwd = cwd
				}
			case arg == "--list":
				logsOptions.listOnly = true
			case strings.HasPrefix(arg, "--list-format="):
				if len(arg[len("--list-format="):]) > 0 {
					logsOptions.listType = arg[len("--list-format="):]
				}
			case arg == "--all":
				logsOptions.includeAll = true
			case strings.HasPrefix(arg, "--sort="):
				if len(arg[len("--sort="):]) > 0 {
					inputSortType := arg[len("--sort="):]
					switch inputSortType {
					case "task":
						unresolvedSortType = TaskSort
					case "duration":
						unresolvedSortType = DurationSort
					case "datetime":
						unresolvedSortType = DatetimeSort
					case "alnum":
						unresolvedSortType = AlnumSort
					default:
						return nil, fmt.Errorf("invalid value %v for --sort CLI flag. This should be task, duration, datetime, or alnum", inputSortType)
					}
				}
			case arg == "--reverse":
				logsOptions.reverseSort = true
			case strings.HasPrefix(arg, "--cache-dir"):
				unresolvedCacheFolder = arg[len("--cache-dir="):]
			case strings.HasPrefix(arg, "--last-run-path="):
				unresolvedLastRunPath = arg[len("--last-run-path="):]
			case strings.HasPrefix(arg, "--output-logs"):
				outputLogsMode := arg[len("--output-logs="):]
				if len(outputLogsMode) > 0 {
					switch outputLogsMode {
					case "full":
						logsOptions.outputLogsMode = outputLogsMode
					case "hash-only":
						logsOptions.outputLogsMode = "hash"
					default:
						output.Warn(fmt.Sprintf("[WARNING] unknown value %v for --output-logs CLI flag. Falling back to full", outputLogsMode))
					}
				}
			default:
				return nil, errors.New(fmt.Sprintf("unknown flag: %v", arg))
			}
		} else if !strings.HasPrefix(arg, "-") {
			if !queryHashesSet.Includes(arg) {
				queryHashesSet.Add(arg)
				logsOptions.queryHashes = append(logsOptions.queryHashes, arg)
			}
		}
	}

	// We can only set sortType once we know what the default should
	//  be and whether or not it has been overridden
	if len(logsOptions.queryHashes) > 0 && unresolvedSortType == NothingSort {
		unresolvedSortType = QuerySort
	}
	if logsOptions.includeAll && unresolvedSortType == NothingSort {
		unresolvedSortType = DatetimeSort
	}
	if unresolvedSortType != NothingSort {
		logsOptions.sortType = unresolvedSortType
	}

	// We can only set this cache folder after we know actual cwd
	logsOptions.cacheFolder = filepath.Join(logsOptions.cwd, unresolvedCacheFolder)
	// We can only set lastRunPath after we know the final cacheFolder path
	//  and whether or not it has been overridden
	if unresolvedLastRunPath == "" {
		unresolvedLastRunPath = filepath.Join(logsOptions.cacheFolder, "last-run.log")
	}
	logsOptions.lastRunPath = unresolvedLastRunPath

	return logsOptions, nil
}

// logWarning logs an error and outputs it to the UI as a warning.
func (c *LogsCommand) logWarning(log hclog.Logger, prefix string, err error) {
	log.Warn(prefix, "warning", err)

	if prefix != "" {
		prefix = " " + prefix + ": "
	}

	c.Ui.Error(fmt.Sprintf("%s%s%s", ui.WARNING_PREFIX, prefix, color.YellowString(" %v", err)))
}

// logError logs an error and outputs it to the UI.
func (c *LogsCommand) logError(log hclog.Logger, prefix string, err error) {
	log.Error(prefix, "error", err)

	if prefix != "" {
		prefix += ": "
	}

	c.Ui.Error(fmt.Sprintf("%s%s%s", ui.ERROR_PREFIX, prefix, color.RedString(" %v", err)))
}

// logInfo logs an info message and outputs it to the UI.
func (c *LogsCommand) logInfo(log hclog.Logger, message string) {
	log.Info(message)

	c.Ui.Info(fmt.Sprintf("%s%s", ui.INFO_PREFIX, color.BlueString(" %v", message)))
}
