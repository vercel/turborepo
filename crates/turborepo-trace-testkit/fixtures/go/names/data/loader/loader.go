package loader

import "os"

// Store reads named documents below a directory.
type Store struct {
	dir string
}

// New returns a store for dir.
func New(dir string) *Store {
	return &Store{dir: dir}
}

// Get reads one document.
func (s *Store) Get(name string) ([]byte, error) {
	return os.ReadFile(s.dir + "/" + name)
}
