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
	"sync"
	"turbo/internal/config"
	"turbo/internal/context"
	"turbo/internal/fs"
	"turbo/internal/ui"
	"turbo/internal/util"

	mapset "github.com/deckarep/golang-set"
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

func parsePruneArgs(args []string) (*PruneOptions, error) {
	var options = &PruneOptions{}

	if len(args) == 0 {
		return nil, errors.Errorf("At least one target must be specified.")
	}

	cwd, err := os.Getwd()
	if err != nil {
		return nil, errors.Errorf("invalid working directory")
	}
	options.cwd = cwd
	for _, arg := range args {
		if strings.HasPrefix(arg, "--") {
			switch {
			case strings.HasPrefix(arg, "--scope="):
				options.scope = arg[len("--scope="):]
			case strings.HasPrefix(arg, "--docker"):
				options.docker = true
			case strings.HasPrefix(arg, "--cwd="):
				if len(arg[len("--cwd="):]) > 1 {
					options.cwd = arg[len("--cwd="):]
				}
			default:
				return nil, errors.New(fmt.Sprintf("unknown flag: %v", arg))
			}
		}
	}

	return options, nil
}

// Prune creates a smaller monorepo with only the required workspaces
func (c *PruneCommand) Run(args []string) int {
	pruneOptions, err := parsePruneArgs(args)
	logger := log.New(os.Stdout, "", 0)
	if err != nil {
		c.logError(c.Config.Logger, "", err)
		return 1
	}
	ctx, err := context.New(context.WithTracer(""), context.WithArgs(args), context.WithGraph(".", c.Config))

	if err != nil {
		c.logError(c.Config.Logger, "", fmt.Errorf("could not construct graph: %w", err))
		return 1
	}
	c.Config.Logger.Trace("scope", "value", pruneOptions.scope)
	target := ctx.PackageInfos[pruneOptions.scope]
	c.Config.Logger.Trace("target", "value", target.Name)
	c.Config.Logger.Trace("directory", "value", target.Dir)
	c.Config.Logger.Trace("external deps", "value", target.UnresolvedExternalDeps)
	c.Config.Logger.Trace("internal deps", "value", target.InternalDeps)
	c.Config.Logger.Trace("docker", "value", pruneOptions.docker)
	c.Config.Logger.Trace("out dir", "value", filepath.Join(pruneOptions.cwd, "out"))

	if !util.IsYarn(ctx.Backend.Name) {
		c.logError(c.Config.Logger, "", fmt.Errorf("this command is not yet implemented for %s", ctx.Backend.Name))
		return 1
	}

	logger.Printf("Generating pruned monorepo for %v in %v", ui.Bold(pruneOptions.scope), ui.Bold(filepath.Join(pruneOptions.cwd, "out")))

	err = fs.EnsureDir(filepath.Join(pruneOptions.cwd, "out", "package.json"))
	if err != nil {
		c.logError(c.Config.Logger, "", fmt.Errorf("could not create directory: %w", err))
		return 1
	}
	workspaces := []string{}
	seen := mapset.NewSet()
	var lockfileWg sync.WaitGroup
	pkg, err := fs.ReadPackageJSON("package.json")
	if err != nil {
		c.logError(c.Config.Logger, "", fmt.Errorf("could not read package.json: %w", err))
		return 1
	}
	depSet := mapset.NewSet()
	pkg.UnresolvedExternalDeps = make(map[string]string)
	for dep, version := range pkg.Dependencies {
		pkg.UnresolvedExternalDeps[dep] = version
	}
	for dep, version := range pkg.DevDependencies {
		pkg.UnresolvedExternalDeps[dep] = version
	}
	for dep, version := range pkg.OptionalDependencies {
		pkg.UnresolvedExternalDeps[dep] = version
	}
	for dep, version := range pkg.PeerDependencies {
		pkg.UnresolvedExternalDeps[dep] = version
	}

	pkg.SubLockfile = make(fs.YarnLockfile)
	ctx.ResolveDepGraph(&lockfileWg, pkg.UnresolvedExternalDeps, depSet, seen, pkg)

	lockfileWg.Wait()
	lockfile := pkg.SubLockfile
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
	c.Config.Logger.Trace("new worksapces", "value", workspaces)
	if pruneOptions.docker {
		if fs.FileExists(".gitignore") {
			if err := fs.CopyFile(".gitignore", filepath.Join(pruneOptions.cwd, "out", "full", ".gitignore"), fs.DirPermissions); err != nil {
				c.logError(c.Config.Logger, "", fmt.Errorf("failed to copy root .gitignore: %w", err))
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
	// because of yarn being yarn, we need to inject lines in between each block of YAML to make it "valid" syml
	f, err := os.Open(filepath.Join(filepath.Join(pruneOptions.cwd, "out", "yarn.lock")))
	if err != nil {
		c.logError(c.Config.Logger, "", fmt.Errorf("failed to massage lockfile: %w", err))
	}
	defer f.Close()

	output, err := os.Create(filepath.Join(pruneOptions.cwd, "out", "yarn-tmp.lock"))
	writer := bufio.NewWriter(output)
	if err != nil {
		c.logError(c.Config.Logger, "", fmt.Errorf("failed create tempory lockfile: %w", err))
	}
	defer output.Close()

	if ctx.Backend.Name == "nodejs-yarn" {
		writer.WriteString("# THIS IS AN AUTOGENERATED FILE. DO NOT EDIT THIS FILE DIRECTLY.\n# yarn lockfile v1\n\n")
	} else {
		writer.WriteString("# This file is generated by running \"yarn install\" inside your project.\n# Manual changes might be lost - proceed with caution!\n\n__metadata:\nversion: 5\ncacheKey: 8\n\n")
	}

	scan := bufio.NewScanner(f)
	buf := make([]byte, 0, 1024*1024)
	scan.Buffer(buf, 10*1024*1024)
	for scan.Scan() {
		line := scan.Text() //Writing to Stdout
		if !strings.HasPrefix(line, " ") {
			writer.WriteString(fmt.Sprintf("\n%v\n", strings.ReplaceAll(line, "'", "\"")))
		} else {
			writer.WriteString(fmt.Sprintf("%v\n", strings.ReplaceAll(line, "'", "\"")))
		}
	}
	writer.Flush() // make sure to flush the log write before we start saving it.

	err = os.Rename(filepath.Join(pruneOptions.cwd, "out", "yarn-tmp.lock"), filepath.Join(pruneOptions.cwd, "out", "yarn.lock"))
	if err != nil {
		c.logError(c.Config.Logger, "", fmt.Errorf("failed finalize lockfile: %w", err))
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
