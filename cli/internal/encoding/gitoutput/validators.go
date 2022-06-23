package gitoutput

import "bytes"

var _allowedObjectType = []byte(" blob tree commit ")
var _allowedStatusChars = []byte(" MTADRCU?!")

// checkValid provides a uniform interface for calling `gitoutput` validators.
func checkValid(fieldType Field, value []byte) error {
	switch fieldType {
	case ObjectMode:
		return checkObjectMode(value)
	case ObjectType:
		return checkObjectType(value)
	case ObjectName:
		return CheckObjectName(value)
	case ObjectStage:
		return checkObjectStage(value)
	case StatusX:
		return checkStatusX(value)
	case StatusY:
		return checkStatusY(value)
	case Path:
		return checkPath(value)
	default:
		return ErrUnknownField
	}
}

// checkObjectMode asserts that a byte slice is a six digit octal string (100644).
// It does not attempt to ensure that the values in particular positions are reasonable.
func checkObjectMode(value []byte) error {
	if len(value) != 6 {
		return ErrInvalidObjectMode
	}

	// 0-7 are 0x30 - 0x37
	for _, currentByte := range value {
		if (currentByte ^ 0x30) > 7 {
			return ErrInvalidObjectMode
		}
	}

	// length of 6, 0-7
	return nil
}

// checkObjectType asserts that a byte slice is a valid possibility (blob, tree, commit).
func checkObjectType(value []byte) error {
	typeLength := len(value)
	// Based upon:
	// min(len("blob"), len("tree"), len("commit"))
	// max(len("blob"), len("tree"), len("commit"))
	if typeLength < 4 || typeLength > 6 {
		return ErrInvalidObjectType
	}

	// Because of the space separator there is no way to pass in a space.
	// We use that trick to enable fast lookups in _allowedObjectType.
	index := bytes.Index(_allowedObjectType, value)

	// Impossible to match at 0, not found is -1.
	if index < 1 {
		return ErrInvalidObjectType
	}

	// Followed by a space.
	if _allowedObjectType[index-1] != byte(_space) {
		return ErrInvalidObjectType
	}

	// Preceded by a space.
	if _allowedObjectType[index+typeLength] != byte(_space) {
		return ErrInvalidObjectType
	}
	return nil
}

// CheckObjectName asserts that a byte slice looks like a SHA hash.
func CheckObjectName(value []byte) error {
	if len(value) != 40 {
		return ErrInvalidObjectName
	}

	// 0-9 are 0x30 - 0x39
	// a-f are 0x61 - 0x66
	for _, currentByte := range value {
		isNumber := (currentByte ^ 0x30) < 10
		numericAlpha := (currentByte ^ 0x60)
		isAlpha := (numericAlpha < 7) && (numericAlpha > 0)
		if !(isNumber || isAlpha) {
			return ErrInvalidObjectName
		}
	}

	// length of 40, hex
	return nil
}

// checkObjectStage asserts that a byte slice is a valid possibility (0-3).
func checkObjectStage(value []byte) error {
	// 0-3 are 0x30 - 0x33
	if len(value) != 1 {
		return ErrInvalidObjectStage
	}

	currentByte := value[0]
	if (currentByte ^ 0x30) >= 4 {
		return ErrInvalidObjectStage
	}

	return nil
}

// checkStatusX asserts that a byte slice is a valid possibility (" MTADRCU?!").
func checkStatusX(value []byte) error {
	if len(value) != 1 {
		return ErrInvalidObjectStatusX
	}

	index := bytes.Index(_allowedStatusChars, value)
	if index == -1 {
		return ErrInvalidObjectStatusX
	}
	return nil
}

// checkStatusY asserts that a byte slice is a valid possibility (" MTADRCU?!").
func checkStatusY(value []byte) error {
	if len(value) != 1 {
		return ErrInvalidObjectStatusY
	}

	index := bytes.Index(_allowedStatusChars, value)
	if index == -1 {
		return ErrInvalidObjectStatusY
	}
	return nil
}

// checkPath asserts that a byte slice is non-empty.
func checkPath(value []byte) error {
	// Exists at all. This is best effort as trying to be fully-compatible is silly.
	if len(value) == 0 {
		return ErrInvalidPath
	}
	return nil
}
