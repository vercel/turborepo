package fs

import (
	"io/fs"
	"os"
)

// LstatCachedFile maintains a cache of file info, mode and type for the given Path
type LstatCachedFile struct {
	Path AbsolutePath
	Info fs.FileInfo
	Mode *fs.FileMode
	Type *fs.FileMode
}

// GetInfo returns, and caches the file info for the LstatCachedFile.Path
func (file *LstatCachedFile) GetInfo() (fs.FileInfo, error) {
	if file.Info != nil {
		return file.Info, nil
	}

	err := file.lstat()
	if err != nil {
		return nil, err
	}

	return file.Info, nil
}

// GetMode returns, and caches the file mode for the LstatCachedFile.Path
func (file *LstatCachedFile) GetMode() (fs.FileMode, error) {
	if file.Mode != nil {
		return *file.Mode, nil
	}

	err := file.lstat()
	if err != nil {
		return 0, err
	}

	return *file.Mode, nil
}

// GetType returns, and caches the file type for the LstatCachedFile.Path
func (file *LstatCachedFile) GetType() (fs.FileMode, error) {
	if file.Type != nil {
		return *file.Type, nil
	}

	err := file.lstat()
	if err != nil {
		return 0, err
	}

	return *file.Type, nil
}

func (file *LstatCachedFile) lstat() error {
	fileInfo, err := file.Path.Lstat()
	if err != nil {
		return err
	}

	fileMode := fileInfo.Mode()
	fileModeType := fileMode & os.ModeType

	file.Info = fileInfo
	file.Mode = &fileMode
	file.Type = &fileModeType

	return nil
}
