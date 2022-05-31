package prune

import (
	"bufio"
	"bytes"
	"fmt"
	"io/ioutil"
	"log"
	"os"
	"path/filepath"
	"strings"

	"github.com/vercel/turborepo/cli/internal/cache"
	"github.com/vercel/turborepo/cli/internal/config"
	"github.com/vercel/turborepo/cli/internal/context"
	"github.com/vercel/turborepo/cli/internal/fs"
	"github.com/vercel/turborepo/cli/internal/ui"
	"github.com/vercel/turborepo/cli/internal/util"

	"github.com/fatih/color"
	"github.com/hashicorp/go-hclog"
	"github.com/mitchellh/cli"
	"github.com/pkg/errors"
	"gopkg.in/yaml.v3"
)

// PruneCommand is a Command implementation that tells Turbo to run a task
type PruneCommand struct {
	Config *config.Config
	Ui     *cli.ColoredUi
}

// Synopsis of run command
func (c *PruneCommand) Synopsis() string {
	return "Prepare a subset of your monorepo"
}

// Help returns information about the `run` command
func (c *PruneCommand) Help() string {
	helpText := `
Usage: turbo prune --scope=<package name>

  Prepare a subset of your monorepo.

Options:
  --help                 Show this screen.
  --scope                Specify package to act as entry point
                         for pruned monorepo (required).
  --docker               Output pruned workspace into 'full'
                         and 'json' directories optimized for
                         Docker layer caching. (default false)
`
	return strings.TrimSpace(helpText)
}

type PruneOptions struct {
	scope  string
	cwd    string
	docker bool
}

func parsePruneArgs(args []string, cwd fs.AbsolutePath) (*PruneOptions, error) {
	var options = &PruneOptions{cwd: cwd.ToStringDuringMigration()}

	if len(args) == 0 {
		return nil, errors.Errorf("At least one target must be specified.")
	}

	for _, arg := range args {
		if strings.HasPrefix(arg, "--") {
			switch {
			case strings.HasPrefix(arg, "--scope="):
				options.scope = arg[len("--scope="):]
			case strings.HasPrefix(arg, "--docker"):
				options.docker = true
			case strings.HasPrefix(arg, "--cwd="):
			default:
				return nil, errors.New(fmt.Sprintf("unknown flag: %v", arg))
			}
		}
	}

	return options, nil
}

// Prune creates a smaller monorepo with only the required workspaces
func (c *PruneCommand) Run(args []string) int {
	pruneOptions, err := parsePruneArgs(args, c.Config.Cwd)
	logger := log.New(os.Stdout, "", 0)
	if err != nil {
		c.logError(c.Config.Logger, "", err)
		return 1
	}
	cacheDir := cache.DefaultLocation(c.Config.Cwd)
	ctx, err := context.New(context.WithGraph(c.Config, cacheDir))

	if err != nil {
		c.logError(c.Config.Logger, "", fmt.Errorf("could not construct graph: %w", err))
		return 1
	}
	c.Config.Logger.Trace("scope", "value", pruneOptions.scope)
	target, scopeIsValid := ctx.PackageInfos[pruneOptions.scope]
	if !scopeIsValid {
		c.logError(c.Config.Logger, "", errors.Errorf("invalid scope: package not found"))
		return 1
	}
	c.Config.Logger.Trace("target", "value", target.Name)
	c.Config.Logger.Trace("directory", "value", target.Dir)
	c.Config.Logger.Trace("external deps", "value", target.UnresolvedExternalDeps)
	c.Config.Logger.Trace("internal deps", "value", target.InternalDeps)
	c.Config.Logger.Trace("docker", "value", pruneOptions.docker)
	c.Config.Logger.Trace("out dir", "value", filepath.Join(pruneOptions.cwd, "out"))

	if !util.IsYarn(ctx.PackageManager.Name) {
		c.logError(c.Config.Logger, "", fmt.Errorf("this command is not yet implemented for %s", ctx.PackageManager.Name))
		return 1
	} else if ctx.PackageManager.Name == "nodejs-berry" {
		isNMLinker, err := util.IsNMLinker(pruneOptions.cwd)
		if err != nil {
			c.logError(c.Config.Logger, "", fmt.Errorf("could not determine if yarn is using `nodeLinker: node-modules`: %w", err))
			return 1
		} else if !isNMLinker {
			c.logError(c.Config.Logger, "", fmt.Errorf("only yarn v2/v3 with `nodeLinker: node-modules` is supported at this time"))
			return 1
		}
	}

	logger.Printf("Generating pruned monorepo for %v in %v", ui.Bold(pruneOptions.scope), ui.Bold(filepath.Join(pruneOptions.cwd, "out")))

	err = fs.EnsureDir(filepath.Join(pruneOptions.cwd, "out", "package.json"))
	if err != nil {
		c.logError(c.Config.Logger, "", fmt.Errorf("could not create directory: %w", err))
		return 1
	}
	workspaces := []string{}
	lockfile := c.Config.RootPackageJSON.SubLockfile
	targets := []interface{}{pruneOptions.scope}
	internalDeps, err := ctx.TopologicalGraph.Ancestors(pruneOptions.scope)
	if err != nil {
		c.logError(c.Config.Logger, "", fmt.Errorf("could find traverse the dependency graph to find topological dependencies: %w", err))
		return 1
	}
	targets = append(targets, internalDeps.List()...)

	for _, internalDep := range targets {
		if internalDep == ctx.RootNode {
			continue
		}
		workspaces = append(workspaces, ctx.PackageInfos[internalDep].Dir)
		if pruneOptions.docker {
			targetDir := filepath.Join(pruneOptions.cwd, "out", "full", ctx.PackageInfos[internalDep].Dir)
			jsonDir := filepath.Join(pruneOptions.cwd, "out", "json", ctx.PackageInfos[internalDep].PackageJSONPath)
			if err := fs.EnsureDir(targetDir); err != nil {
				c.logError(c.Config.Logger, "", fmt.Errorf("failed to create folder %v for %v: %w", targetDir, internalDep, err))
				return 1
			}
			if err := fs.RecursiveCopy(ctx.PackageInfos[internalDep].Dir, targetDir, fs.DirPermissions); err != nil {
				c.logError(c.Config.Logger, "", fmt.Errorf("failed to copy %v into %v: %w", internalDep, targetDir, err))
				return 1
			}
			if err := fs.EnsureDir(jsonDir); err != nil {
				c.logError(c.Config.Logger, "", fmt.Errorf("failed to create folder %v for %v: %w", jsonDir, internalDep, err))
				return 1
			}
			if err := fs.RecursiveCopy(ctx.PackageInfos[internalDep].PackageJSONPath, jsonDir, fs.DirPermissions); err != nil {
				c.logError(c.Config.Logger, "", fmt.Errorf("failed to copy %v into %v: %w", internalDep, jsonDir, err))
				return 1
			}
		} else {
			targetDir := filepath.Join(pruneOptions.cwd, "out", ctx.PackageInfos[internalDep].Dir)
			if err := fs.EnsureDir(targetDir); err != nil {
				c.logError(c.Config.Logger, "", fmt.Errorf("failed to create folder %v for %v: %w", targetDir, internalDep, err))
				return 1
			}
			if err := fs.RecursiveCopy(ctx.PackageInfos[internalDep].Dir, targetDir, fs.DirPermissions); err != nil {
				c.logError(c.Config.Logger, "", fmt.Errorf("failed to copy %v into %v: %w", internalDep, targetDir, err))
				return 1
			}
		}

		for k, v := range ctx.PackageInfos[internalDep].SubLockfile {
			lockfile[k] = v
		}

		logger.Printf(" - Added %v", ctx.PackageInfos[internalDep].Name)
	}
	c.Config.Logger.Trace("new workspaces", "value", workspaces)
	if pruneOptions.docker {
		if fs.FileExists(".gitignore") {
			if err := fs.CopyFile(".gitignore", filepath.Join(pruneOptions.cwd, "out", "full", ".gitignore"), fs.DirPermissions); err != nil {
				c.logError(c.Config.Logger, "", fmt.Errorf("failed to copy root .gitignore: %w", err))
				return 1
			}
		}
		// We only need to actually copy turbo.json into "full" folder since it isn't needed for installation in docker
		if fs.FileExists("turbo.json") {
			if err := fs.CopyFile("turbo.json", filepath.Join(pruneOptions.cwd, "out", "full", "turbo.json"), fs.DirPermissions); err != nil {
				c.logError(c.Config.Logger, "", fmt.Errorf("failed to copy root turbo.json: %w", err))
				return 1
			}
		}

		if err := fs.CopyFile("package.json", filepath.Join(pruneOptions.cwd, "out", "full", "package.json"), fs.DirPermissions); err != nil {
			c.logError(c.Config.Logger, "", fmt.Errorf("failed to copy root package.json: %w", err))
			return 1
		}

		if err := fs.CopyFile("package.json", filepath.Join(pruneOptions.cwd, "out", "json", "package.json"), fs.DirPermissions); err != nil {
			c.logError(c.Config.Logger, "", fmt.Errorf("failed to copy root package.json: %w", err))
			return 1
		}
	} else {
		if fs.FileExists(".gitignore") {
			if err := fs.CopyFile(".gitignore", filepath.Join(pruneOptions.cwd, "out", ".gitignore"), fs.DirPermissions); err != nil {
				c.logError(c.Config.Logger, "", fmt.Errorf("failed to copy root .gitignore: %w", err))
				return 1
			}
		}

		if fs.FileExists("turbo.json") {
			if err := fs.CopyFile("turbo.json", filepath.Join(pruneOptions.cwd, "out", "turbo.json"), fs.DirPermissions); err != nil {
				c.logError(c.Config.Logger, "", fmt.Errorf("failed to copy root turbo.json: %w", err))
				return 1
			}
		}

		if err := fs.CopyFile("package.json", filepath.Join(pruneOptions.cwd, "out", "package.json"), fs.DirPermissions); err != nil {
			c.logError(c.Config.Logger, "", fmt.Errorf("failed to copy root package.json: %w", err))
			return 1
		}
	}

	var b bytes.Buffer
	yamlEncoder := yaml.NewEncoder(&b)
	yamlEncoder.SetIndent(2) // this is what you're looking for
	yamlEncoder.Encode(lockfile)

	if err != nil {
		c.logError(c.Config.Logger, "", fmt.Errorf("failed to materialize sub-lockfile. This can happen if your lockfile contains merge conflicts or is somehow corrupted. Please report this if it occurs: %w", err))
		return 1
	}
	err = ioutil.WriteFile(filepath.Join(pruneOptions.cwd, "out", "yarn.lock"), b.Bytes(), fs.DirPermissions)
	if err != nil {
		c.logError(c.Config.Logger, "", fmt.Errorf("failed to write sub-lockfile: %w", err))
		return 1
	}

	tmpGeneratedLockfile, err := os.Create(filepath.Join(pruneOptions.cwd, "out", "yarn-tmp.lock"))
	tmpGeneratedLockfileWriter := bufio.NewWriter(tmpGeneratedLockfile)
	if err != nil {
		c.logError(c.Config.Logger, "", fmt.Errorf("failed create temporary lockfile: %w", err))
		return 1
	}

	if ctx.PackageManager.Name == "nodejs-yarn" {
		tmpGeneratedLockfileWriter.WriteString("# THIS IS AN AUTOGENERATED FILE. DO NOT EDIT THIS FILE DIRECTLY.\n# yarn lockfile v1\n\n")
	} else {
		tmpGeneratedLockfileWriter.WriteString("# This file is generated by running \"yarn install\" inside your project.\n# Manual changes might be lost - proceed with caution!\n\n__metadata:\nversion: 5\ncacheKey: 8\n\n")
	}

	// because of yarn being yarn, we need to inject lines in between each block of YAML to make it "valid" SYML
	generatedLockfile, err := os.Open(filepath.Join(filepath.Join(pruneOptions.cwd, "out", "yarn.lock")))
	if err != nil {
		c.logError(c.Config.Logger, "", fmt.Errorf("failed to massage lockfile: %w", err))
		return 1
	}

	scan := bufio.NewScanner(generatedLockfile)
	buf := make([]byte, 0, 1024*1024)
	scan.Buffer(buf, 10*1024*1024)
	for scan.Scan() {
		line := scan.Text() //Writing to Stdout
		if !strings.HasPrefix(line, " ") {
			tmpGeneratedLockfileWriter.WriteString(fmt.Sprintf("\n%v\n", strings.ReplaceAll(line, "'", "\"")))
		} else {
			tmpGeneratedLockfileWriter.WriteString(fmt.Sprintf("%v\n", strings.ReplaceAll(line, "'", "\"")))
		}
	}
	// Make sure to flush the log write before we start saving it.
	tmpGeneratedLockfileWriter.Flush()

	// Close the files before we rename them
	tmpGeneratedLockfile.Close()
	generatedLockfile.Close()

	// Rename the file
	err = os.Rename(filepath.Join(pruneOptions.cwd, "out", "yarn-tmp.lock"), filepath.Join(pruneOptions.cwd, "out", "yarn.lock"))
	if err != nil {
		c.logError(c.Config.Logger, "", fmt.Errorf("failed finalize lockfile: %w", err))
		return 1
	}
	return 0
}

// logError logs an error and outputs it to the UI.
func (c *PruneCommand) logError(log hclog.Logger, prefix string, err error) {
	log.Error(prefix, "error", err)

	if prefix != "" {
		prefix += ": "
	}
	pref := color.New(color.Bold, color.FgRed, color.ReverseVideo).Sprint(" ERROR ")
	c.Ui.Error(fmt.Sprintf("%s%s%s", pref, prefix, color.RedString(" %v", err)))
}
