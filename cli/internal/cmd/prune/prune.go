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

	"github.com/spf13/cobra"
	"github.com/vercel/turborepo/cli/internal/cmdutil"
	"github.com/vercel/turborepo/cli/internal/context"
	"github.com/vercel/turborepo/cli/internal/fs"
	"github.com/vercel/turborepo/cli/internal/ui"
	"github.com/vercel/turborepo/cli/internal/util"
	"gopkg.in/yaml.v3"
)

func PruneCmd(ch *cmdutil.Helper) *cobra.Command {
	var opts struct {
		scope  string
		docker bool
		cwd    string
	}

	cmd := &cobra.Command{
		Use:   "prune",
		Short: "Prepare a subset of your monorepo",
		RunE: func(cmd *cobra.Command, args []string) error {
			logger := log.New(os.Stdout, "", 0)

			ctx, err := context.New(context.WithGraph(opts.cwd, ch.Config))
			if err != nil {
				return ch.LogError("could not construct graph: %w", err)
			}

			ch.Config.Logger.Trace("scope", "value", opts.scope)
			target, scopeIsValid := ctx.PackageInfos[opts.scope]
			if !scopeIsValid {
				return ch.LogError("invalid scope: package not found")
			}

			ch.Config.Logger.Trace("target", "value", target.Name)
			ch.Config.Logger.Trace("directory", "value", target.Dir)
			ch.Config.Logger.Trace("external deps", "value", target.UnresolvedExternalDeps)
			ch.Config.Logger.Trace("internal deps", "value", target.InternalDeps)
			ch.Config.Logger.Trace("docker", "value", opts.docker)
			ch.Config.Logger.Trace("out dir", "value", filepath.Join(opts.cwd, "out"))

			if !util.IsYarn(ctx.Backend.Name) {
				return ch.LogError("this command is not yet implemented for %s", ctx.Backend.Name)
			} else if ctx.Backend.Name == "nodejs-berry" {
				isNMLinker, err := util.IsNMLinker(opts.cwd)
				if err != nil {
					return ch.LogError("could not determine if yarn is using `nodeLinker: node-modules`: %w", err)
				} else if !isNMLinker {
					return ch.LogError("only yarn v2/v3 with `nodeLinker: node-modules` is supported at this time")
				}
			}

			logger.Printf("Generating pruned monorepo for %v in %v", ui.Bold(opts.scope), ui.Bold(filepath.Join(opts.cwd, "out")))

			err = fs.EnsureDir(filepath.Join(opts.cwd, "out", "package.json"))
			if err != nil {
				return ch.LogError("could not create directory: %w", err)
			}
			workspaces := []string{}
			lockfile := ctx.RootPackageInfo.SubLockfile
			targets := []interface{}{opts.scope}
			internalDeps, err := ctx.TopologicalGraph.Ancestors(opts.scope)
			if err != nil {
				return ch.LogError("could find traverse the dependency graph to find topological dependencies: %w", err)
			}
			targets = append(targets, internalDeps.List()...)

			for _, internalDep := range targets {
				if internalDep == ctx.RootNode {
					continue
				}
				workspaces = append(workspaces, ctx.PackageInfos[internalDep].Dir)
				if opts.docker {
					targetDir := filepath.Join(opts.cwd, "out", "full", ctx.PackageInfos[internalDep].Dir)
					jsonDir := filepath.Join(opts.cwd, "out", "json", ctx.PackageInfos[internalDep].PackageJSONPath)
					if err := fs.EnsureDir(targetDir); err != nil {
						return ch.LogError("failed to create folder %v for %v: %w", targetDir, internalDep, err)
					}
					if err := fs.RecursiveCopy(ctx.PackageInfos[internalDep].Dir, targetDir, fs.DirPermissions); err != nil {
						return ch.LogError("failed to copy %v into %v: %w", internalDep, targetDir, err)
					}
					if err := fs.EnsureDir(jsonDir); err != nil {
						return ch.LogError("failed to create folder %v for %v: %w", jsonDir, internalDep, err)
					}
					if err := fs.RecursiveCopy(ctx.PackageInfos[internalDep].PackageJSONPath, jsonDir, fs.DirPermissions); err != nil {
						return ch.LogError("failed to copy %v into %v: %w", internalDep, jsonDir, err)
					}
				} else {
					targetDir := filepath.Join(opts.cwd, "out", ctx.PackageInfos[internalDep].Dir)
					if err := fs.EnsureDir(targetDir); err != nil {
						return ch.LogError("failed to create folder %v for %v: %w", targetDir, internalDep, err)
					}
					if err := fs.RecursiveCopy(ctx.PackageInfos[internalDep].Dir, targetDir, fs.DirPermissions); err != nil {
						return ch.LogError("failed to copy %v into %v: %w", internalDep, targetDir, err)
					}
				}

				for k, v := range ctx.PackageInfos[internalDep].SubLockfile {
					lockfile[k] = v
				}

				logger.Printf(" - Added %v", ctx.PackageInfos[internalDep].Name)
			}
			ch.Config.Logger.Trace("new workspaces", "value", workspaces)
			if opts.docker {
				if fs.FileExists(".gitignore") {
					if err := fs.CopyFile(".gitignore", filepath.Join(opts.cwd, "out", "full", ".gitignore"), fs.DirPermissions); err != nil {
						return ch.LogError("failed to copy root .gitignore: %w", err)
					}
				}
				// We only need to actually copy turbo.json into "full" folder since it isn't needed for installation in docker
				if fs.FileExists("turbo.json") {
					if err := fs.CopyFile("turbo.json", filepath.Join(opts.cwd, "out", "full", "turbo.json"), fs.DirPermissions); err != nil {
						return ch.LogError("failed to copy root turbo.json: %w", err)
					}
				}

				if err := fs.CopyFile("package.json", filepath.Join(opts.cwd, "out", "full", "package.json"), fs.DirPermissions); err != nil {
					return ch.LogError("failed to copy root package.json: %w", err)
				}

				if err := fs.CopyFile("package.json", filepath.Join(opts.cwd, "out", "json", "package.json"), fs.DirPermissions); err != nil {
					return ch.LogError("failed to copy root package.json: %w", err)
				}
			} else {
				if fs.FileExists(".gitignore") {
					if err := fs.CopyFile(".gitignore", filepath.Join(opts.cwd, "out", ".gitignore"), fs.DirPermissions); err != nil {
						return ch.LogError("failed to copy root .gitignore: %w", err)
					}
				}

				if fs.FileExists("turbo.json") {
					if err := fs.CopyFile("turbo.json", filepath.Join(opts.cwd, "out", "turbo.json"), fs.DirPermissions); err != nil {
						return ch.LogError("failed to copy root turbo.json: %w", err)
					}
				}

				if err := fs.CopyFile("package.json", filepath.Join(opts.cwd, "out", "package.json"), fs.DirPermissions); err != nil {
					return ch.LogError("failed to copy root package.json: %w", err)
				}
			}

			var b bytes.Buffer
			yamlEncoder := yaml.NewEncoder(&b)
			yamlEncoder.SetIndent(2) // this is what you're looking for
			yamlEncoder.Encode(lockfile)

			if err != nil {
				return ch.LogError("failed to materialize sub-lockfile. This can happen if your lockfile contains merge conflicts or is somehow corrupted. Please report this if it occurs: %w", err)
			}
			err = ioutil.WriteFile(filepath.Join(opts.cwd, "out", "yarn.lock"), b.Bytes(), fs.DirPermissions)
			if err != nil {
				return ch.LogError("failed to write sub-lockfile: %w", err)
			}

			tmpGeneratedLockfile, err := os.Create(filepath.Join(opts.cwd, "out", "yarn-tmp.lock"))
			tmpGeneratedLockfileWriter := bufio.NewWriter(tmpGeneratedLockfile)
			if err != nil {
				return ch.LogError("failed create temporary lockfile: %w", err)
			}

			if ctx.Backend.Name == "nodejs-yarn" {
				tmpGeneratedLockfileWriter.WriteString("# THIS IS AN AUTOGENERATED FILE. DO NOT EDIT THIS FILE DIRECTLY.\n# yarn lockfile v1\n\n")
			} else {
				tmpGeneratedLockfileWriter.WriteString("# This file is generated by running \"yarn install\" inside your project.\n# Manual changes might be lost - proceed with caution!\n\n__metadata:\nversion: 5\ncacheKey: 8\n\n")
			}

			// because of yarn being yarn, we need to inject lines in between each block of YAML to make it "valid" SYML
			generatedLockfile, err := os.Open(filepath.Join(filepath.Join(opts.cwd, "out", "yarn.lock")))
			if err != nil {
				return ch.LogError("failed to massage lockfile: %w", err)
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
			err = os.Rename(filepath.Join(opts.cwd, "out", "yarn-tmp.lock"), filepath.Join(opts.cwd, "out", "yarn.lock"))
			if err != nil {
				return ch.LogError("failed finalize lockfile: %w", err)
			}

			return nil
		},
	}

	path, err := os.Getwd()
	if err != nil {
		return nil
	}

	cmd.Flags().StringVar(&opts.scope, "scope", "", "package to act as entry point for pruned monorepo")
	cmd.Flags().BoolVarP(&opts.docker, "docker", "d", false, "output pruned workspace into 'full' and 'json' directories optimized for Docker layer caching")
	cmd.Flags().StringVar(&opts.cwd, "cwd", path, "directory to execute command in")

	cmd.MarkFlagRequired("scope")

	return cmd
}
