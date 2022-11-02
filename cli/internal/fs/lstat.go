package fs

import (
	"io/fs"
	"os"

	"github.com/vercel/turbo/cli/internal/turbopath"
)

// LstatCachedFile maintains a cache of file info, mode and type for the given Path
type LstatCachedFile struct {
	Path     turbopath.AbsoluteSystemPath
	fileInfo fs.FileInfo
	fileMode *fs.FileMode
	fileType *fs.FileMode
}

// GetInfo returns, and caches the file info for the LstatCachedFile.Path
func (file *LstatCachedFile) GetInfo() (fs.FileInfo, error) {
	if file.fileInfo != nil {
		return file.fileInfo, nil
	}

	err := file.lstat()
	if err != nil {
		return nil, err
	}

	return file.fileInfo, nil
}

// GetMode returns, and caches the file mode for the LstatCachedFile.Path
func (file *LstatCachedFile) GetMode() (fs.FileMode, error) {
	if file.fileMode != nil {
		return *file.fileMode, nil
	}

	err := file.lstat()
	if err != nil {
		return 0, err
	}

	return *file.fileMode, nil
}

// GetType returns, and caches the type bits of (FileMode & os.ModeType) for the LstatCachedFile.Path
func (file *LstatCachedFile) GetType() (fs.FileMode, error) {
	if file.fileType != nil {
		return *file.fileType, nil
	}

	err := file.lstat()
	if err != nil {
		return 0, err
	}

	return *file.fileType, nil
}

func (file *LstatCachedFile) lstat() error {
	fileInfo, err := file.Path.Lstat()
	if err != nil {
		return err
	}

	fileMode := fileInfo.Mode()
	fileModeType := fileMode & os.ModeType

	file.fileInfo = fileInfo
	file.fileMode = &fileMode
	file.fileType = &fileModeType

	return nil
}
