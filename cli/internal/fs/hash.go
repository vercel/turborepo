package fs

import (
	"crypto/sha1"
	"encoding/hex"
	"fmt"
	"io"
	"os"
	"strconv"
	"turbo/internal/xxhash"
)

func HashObject(i interface{}) (string, error) {
	hash := xxhash.New()

	_, err := hash.Write([]byte(fmt.Sprintf("%v", i)))

	return hex.EncodeToString(hash.Sum(nil)), err
}

func HashFile(filePath string) (string, error) {
	file, err := os.Open(filePath)
	defer file.Close()
	if err != nil {
		return "", err
	}

	hash := xxhash.New()
	if _, err := io.Copy(hash, file); err != nil {
		return "", err
	}

	return hex.EncodeToString(hash.Sum(nil)), nil
}

// GitLikeHashFile is a function that mimics how Git
// calculates the SHA1 for a file (or, in Git terms, a "blob") (without git)
func GitLikeHashFile(filePath string) (string, error) {
	file, err := os.Open(filePath)
	defer file.Close()
	if err != nil {
		return "", err
	}
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
