// Copyright 2020 Google LLC

// Use of this source code is governed by a BSD-style
// license that can be found in the LICENSE file or at
// https://developers.google.com/open-source/licenses/bsd

// Package glob provides equivalent functionality to filepath.Glob while
// meeting different performance requirements.
package globby

import (
	"context"
	"io"
	"os"
	"path/filepath"
	"runtime"
	"strings"
)

// Glob is similar to filepath.Glob but with different performance concerns.
//
// Firstly, It can be canceled via the context. Secondly, it makes no guarantees
// about the order of returned matches. This change allows it to run in O(d+m)
// memory and O(n) time, where m is the number of match results, d is the depth
// of the directory tree the pattern is concerned with, and n is the number of
// files in that tree.
func Glob(ctx context.Context, pattern string) ([]string, error) {
	gr := Stream(pattern)
	ctx, cancel := context.WithCancel(ctx)
	go func() {
		<-ctx.Done()
		gr.Close()
	}()
	defer cancel()

	ret := make([]string, 0)
	for {
		match, err := gr.Next()
		if err != nil {
			return nil, err
		}
		if match == "" {
			break
		}
		ret = append(ret, match)
	}
	return ret, nil
}

// Result is a stream of results from globbing against a pattern.
type Result struct {
	errors  chan error
	results chan string
	cancel  context.CancelFunc
}

// Stream Returns a Result from which glob matches can be streamed.
//
// Stream supports the same pattern syntax and produces the same matches as Go's
// filepath.Glob, but makes no ordering guarantees.
func Stream(pattern string) Result {
	ctx, cancel := context.WithCancel(context.Background())
	g := Result{
		errors:  make(chan error),
		results: make(chan string),
		cancel:  cancel,
	}
	go func() {
		defer close(g.results)
		defer close(g.errors)
		if err := stream(pattern, g.results, ctx.Done()); err != nil {
			g.errors <- err
		}
	}()
	return g
}

// Next returns the next match from the pattern. It returns an empty string when
// the matches are exhausted.
func (g *Result) Next() (string, error) {
	// Note: Next never returns filepath.ErrBadPattern if it has previously
	// returned a match. This isn't specified but it's highly desirable in
	// terms of least-surprise. I don't think there's a concise way for this
	// comment to justify this claim; you have to just read `stream` and
	// `filepath.Match` to convince yourself.
	select {
	case err := <-g.errors:
		g.Close()
		return "", err
	case r := <-g.results:
		return r, nil
	}
}

// Close cancels the in-progress globbing and cleans up. You can call this any
// time, including concurrently with Next. You don't need to call it if Next has
// returned an empty string.
func (g *Result) Close() error {
	g.cancel()
	for _ = range g.errors {
	}
	for _ = range g.results {
	}
	return nil
}

// stream finds files matching pattern and sends their paths on the results
// channel. It stops (returning nil) if the cancel channel is closed.
// The caller must drain the results channel.
func stream(pattern string, results chan<- string, cancel <-chan struct{}) error {
	if !hasMeta(pattern) {
		if _, err := os.Lstat(pattern); err != nil {
			return nil
		}
		results <- pattern
		return nil
	}

	dir, file := filepath.Split(pattern)
	volumeLen := 0
	if runtime.GOOS == "windows" {
		volumeLen, dir = cleanGlobPathWindows(dir)
	} else {
		dir = cleanGlobPath(dir)
	}

	if !hasMeta(dir[volumeLen:]) {
		return glob(dir, file, results, cancel)
	}

	// Prevent infinite recursion. See Go issue 15879.
	if dir == pattern {
		return filepath.ErrBadPattern
	}

	dirMatches := make(chan string)
	var streamErr error
	go func() {
		streamErr = stream(dir, dirMatches, cancel)
		close(dirMatches)
	}()

	for d := range dirMatches {
		if err := glob(d, file, results, cancel); err != nil {
			// Drain channel before returning
			for range dirMatches {
			}
			return err
		}
	}

	return streamErr
}

// cleanGlobPath prepares path for glob matching.
func cleanGlobPath(path string) string {
	switch path {
	case "":
		return "."
	case string(filepath.Separator):
		// do nothing to the path
		return path
	default:
		return path[0 : len(path)-1] // chop off trailing separator
	}
}

// cleanGlobPathWindows is windows version of cleanGlobPath.
func cleanGlobPathWindows(path string) (prefixLen int, cleaned string) {
	vollen := len(filepath.VolumeName(path))
	switch {
	case path == "":
		return 0, "."
	case vollen+1 == len(path) && os.IsPathSeparator(path[len(path)-1]): // /, \, C:\ and C:/
		// do nothing to the path
		return vollen + 1, path
	case vollen == len(path) && len(path) == 2: // C:
		return vollen, path + "." // convert C: into C:.
	default:
		if vollen >= len(path) {
			vollen = len(path) - 1
		}
		return vollen, path[0 : len(path)-1] // chop off trailing separator
	}
}

// glob searches for files matching pattern in the directory dir
// and sends them down the results channel. It stops if the cancel channel is
// closed.
func glob(dir, pattern string, results chan<- string, cancel <-chan struct{}) error {
	fi, err := os.Stat(dir)
	if err != nil {
		return nil
	}
	if !fi.IsDir() {
		return nil
	}
	d, err := os.Open(dir)
	if err != nil {
		return err
	}
	defer d.Close()

	for {
		select {
		case <-cancel:
			return nil
		default:
		}

		names, err := d.Readdirnames(1)
		if err == io.EOF {
			return nil
		}
		if err != nil {
			return err
		}
		n := names[0]

		matched, err := filepath.Match(pattern, n)
		if err != nil {
			return err
		}
		if matched {
			select {
			case results <- filepath.Join(dir, n):
			case <-cancel:
				return nil
			}
		}
	}
}

// hasMeta reports whether path contains any of the magic characters
// recognized by filepath.Match.
func hasMeta(path string) bool {
	magicChars := `*?[`
	if runtime.GOOS != "windows" {
		magicChars = `*?[\`
	}
	return strings.ContainsAny(path, magicChars)
}
