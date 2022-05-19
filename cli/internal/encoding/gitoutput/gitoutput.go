// Package gitoutput reads the output of calls to `git`.
package gitoutput

import (
	"bufio"
	"bytes"
	"errors"
	"fmt"
	"io"
)

type field int

const (
	// ObjectMode is the mode field from `git`` outputs. e.g. 100644
	ObjectMode field = 1
	// ObjectType is the set of allowed types from `git`` outputs: blob, tree, commit
	ObjectType field = 2
	// ObjectName is the 40-character SHA hash
	ObjectName field = 3
	// ObjectStage is a value 0-3.
	ObjectStage field = 4
	// StatusX is the first character of the two-character output from `git status`.
	StatusX field = 5
	// StatusY is the second character of the two-character output from `git status`.
	StatusY field = 6
	// Path is the file path under version control in `git`.
	Path field = 7
)

// Separators that appear in the output of `git`
const space rune = ' '
const tab rune = '\t'
const nul rune = '\000'

// A ParseError is returned for parsing errors.
// Entries and columns are both 1-indexed.
type ParseError struct {
	Entry  int   // Entry where the error occurred
	Column int   // Column where the error occurred
	Err    error // The actual error
}

func (e *ParseError) Error() string {
	return fmt.Sprintf("parse error on entry %d, column %d: %v", e.Entry, e.Column, e.Err)
}

func (e *ParseError) Unwrap() error { return e.Err }

// These are the errors that can be returned in ParseError.Err.
var (
	ErrInvalidObjectMode    = errors.New("object mode is not valid")
	ErrInvalidObjectType    = errors.New("object type is not valid")
	ErrInvalidObjectName    = errors.New("object name is not valid")
	ErrInvalidObjectStage   = errors.New("object stage is not valid")
	ErrInvalidObjectStatusX = errors.New("object status x is not valid")
	ErrInvalidObjectStatusY = errors.New("object status y is not valid")
	ErrInvalidPath          = errors.New("path is not valid")
	ErrFieldCount           = errors.New("too many fields")
)

// A Reader reads records from `git`'s output`.
type Reader struct {
	// ReuseRecord controls whether calls to Read may return a slice sharing
	// the backing array of the previous call's returned slice for performance.
	// By default, each call to Read returns newly allocated memory owned by the caller.
	ReuseRecord bool

	// Fields specifies the type of each field.
	Fields []field

	reader *bufio.Reader

	// numEntry is the current entry being read in the `git` output.
	numEntry int

	// rawBuffer is an entry buffer only used by the readEntry method.
	rawBuffer []byte

	// recordBuffer holds the unescaped fields, one after another.
	// The fields can be accessed by using the indexes in fieldIndexes.
	recordBuffer []byte

	// fieldIndexes is an index of fields inside recordBuffer.
	// The i'th field ends at offset fieldIndexes[i] in recordBuffer.
	fieldIndexes []int

	// fieldPositions is an index of field positions for the
	// last record returned by Read.
	fieldPositions []position

	// lastRecord is a record cache and only used when ReuseRecord == true.
	lastRecord []string
}

// NewLSTreeReader returns a new Reader that reads from reader.
func NewLSTreeReader(reader io.Reader) *Reader {
	return &Reader{
		reader: bufio.NewReader(reader),
		Fields: []field{ObjectMode, ObjectType, ObjectName, Path},
	}
}

// NewLSFilesReader returns a new Reader that reads from reader.
func NewLSFilesReader(reader io.Reader) *Reader {
	return &Reader{
		reader: bufio.NewReader(reader),
		Fields: []field{ObjectMode, ObjectName, ObjectStage, Path},
	}
}

// NewStatusReader returns a new Reader that reads from reader.
func NewStatusReader(reader io.Reader) *Reader {
	return &Reader{
		reader: bufio.NewReader(reader),
		Fields: []field{StatusX, StatusY, Path},
	}
}

// Read reads one record from `reader`.
// Read always returns either a non-nil record or a non-nil error,
// but not both.
//
// If there is no data left to be read, Read returns nil, io.EOF.
//
// If ReuseRecord is true, the returned slice may be shared
// between multiple calls to Read.
func (r *Reader) Read() (record []string, err error) {
	if r.ReuseRecord {
		record, err = r.readRecord(r.lastRecord)
		r.lastRecord = record
	} else {
		record, err = r.readRecord(nil)
	}
	return record, err
}

// FieldPos returns the entry and column corresponding to
// the start of the field with the given index in the slice most recently
// returned by Read. Numbering of entries and columns starts at 1;
// columns are counted in bytes, not runes.
//
// If this is called with an out-of-bounds index, it panics.
func (r *Reader) FieldPos(field int) (entry int, column int) {
	if field < 0 || field >= len(r.fieldPositions) {
		panic("out of range index passed to FieldPos")
	}
	p := &r.fieldPositions[field]
	return p.entry, p.col
}

// pos holds the position of a field in the current entry.
type position struct {
	entry, col int
}

// ReadAll reads all the records from reader until EOF.
//
// A successful call returns err == nil, not err == io.EOF. Because ReadAll is
// defined to read until EOF, it does not treat end of file as an error to be
// reported.
func (r *Reader) ReadAll() (records [][]string, err error) {
	for {
		record, err := r.readRecord(nil)
		if err == io.EOF {
			return records, nil
		}
		if err != nil {
			return nil, err
		}
		records = append(records, record)
	}
}

// readEntry reads the next entry (with the trailing NUL).
// If EOF is hit without a trailing NUL, it will be omitted.
// If some bytes were read then the error is never io.EOF.
// The result is only valid until the next call to readEntry.
func (r *Reader) readEntry() ([]byte, error) {
	entry, err := r.reader.ReadSlice('\000')
	if err == bufio.ErrBufferFull {
		r.rawBuffer = append(r.rawBuffer[:0], entry...)
		for err == bufio.ErrBufferFull {
			entry, err = r.reader.ReadSlice('\000')
			r.rawBuffer = append(r.rawBuffer, entry...)
		}
		entry = r.rawBuffer
	}
	if len(entry) > 0 && err == io.EOF {
		entry = append(entry, '\000')
		err = nil
	}
	r.numEntry++

	return entry, err
}

func getFieldLength(fieldType field, fieldNumber int, fieldCount int, entry *[]byte) (int, int) {
	switch fieldType {
	case StatusX:
		return 1, 0
	case StatusY:
		return 1, 1
	default:
		// TODO: Make sure it isn't past a different separator.
		return bytes.IndexRune(*entry, getSeparator(fieldNumber, fieldCount)), 1
	}
}

func getSeparator(fieldNumber int, fieldCount int) rune {
	remaining := fieldCount - fieldNumber

	switch remaining {
	default:
		return space
	case 2:
		return tab
	case 1:
		return nul
	}
}

// readRecord reads a single record.
func (r *Reader) readRecord(dst []string) ([]string, error) {
	entry, errRead := r.readEntry()
	if errRead == io.EOF {
		return nil, errRead
	}

	// Parse each field in the record.
	r.recordBuffer = r.recordBuffer[:0]
	r.fieldIndexes = r.fieldIndexes[:0]
	r.fieldPositions = r.fieldPositions[:0]
	pos := position{entry: r.numEntry, col: 1}

	fieldCount := len(r.Fields)

	for fieldNumber, fieldType := range r.Fields {
		length, advance := getFieldLength(fieldType, fieldNumber, fieldCount, &entry)
		field := entry[:length]

		fieldError := checkValid(fieldType, &field)
		if fieldError != nil {
			return nil, fieldError
		}

		offset := length + advance
		entry = entry[offset:]
		r.recordBuffer = append(r.recordBuffer, field...)
		r.fieldIndexes = append(r.fieldIndexes, len(r.recordBuffer))
		r.fieldPositions = append(r.fieldPositions, pos)
		pos.col += offset
	}

	// Create a single string and create slices out of it.
	// This pins the memory of the fields together, but allocates once.
	str := string(r.recordBuffer) // Convert to string once to batch allocations
	dst = dst[:0]
	if cap(dst) < len(r.fieldIndexes) {
		dst = make([]string, len(r.fieldIndexes))
	}
	dst = dst[:len(r.fieldIndexes)]
	var preIdx int
	for i, idx := range r.fieldIndexes {
		dst[i] = str[preIdx:idx]
		preIdx = idx
	}

	return dst, nil
}
