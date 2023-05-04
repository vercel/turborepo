package fs

import (
	"crypto/sha1"
	"encoding/hex"
	"fmt"
	"io"
	"strconv"

	"github.com/vercel/turbo/cli/internal/turbopath"
	"github.com/vercel/turbo/cli/internal/xxhash"
)

func HashObject(i interface{}) (string, error) {
	hash := xxhash.New()

	_, err := hash.Write([]byte(fmt.Sprintf("%v", i)))

	return hex.EncodeToString(hash.Sum(nil)), err
}

// GitLikeHashFile is a function that mimics how Git
// calculates the SHA1 for a file (or, in Git terms, a "blob") (without git)
func GitLikeHashFile(filePath turbopath.AbsoluteSystemPath) (string, error) {
	file, err := filePath.Open()
	if err != nil {
		return "", err
	}
	defer file.Close()

	stat, err := file.Stat()
	if err != nil {
		return "", err
	}
	hash := sha1.New()
	hash.Write([]byte("blob"))
	hash.Write([]byte(" "))
	hash.Write([]byte(strconv.FormatInt(stat.Size(), 10)))
	hash.Write([]byte{0})

	if _, err := io.Copy(hash, file); err != nil {
		return "", err
	}

	return hex.EncodeToString(hash.Sum(nil)), nil
}
