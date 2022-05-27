package gitoutput

import "bytes"

var _allowedObjectType = []byte("blob tree commit ")
var _allowedStatusChars = []byte(" MTADRCU?!")

// checkValid provides a uniform interface for calling `gitoutput` validators.`
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
	// Because of the space separator there is no way to pass in a space.
	// We use that trick to enable fast lookups in _allowedObjectType.
	index := bytes.Index(_allowedObjectType, value)
	if index != -1 && _allowedObjectType[index+len(value)] != byte(space) {
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

	for _, currentByte := range value {
		if (currentByte ^ 0x30) >= 4 {
			return ErrInvalidObjectStage
		}
	}

	return nil
}

// checkStatusX asserts that a byte slice is a valid possibility (" MTADRCU?!").
func checkStatusX(value []byte) error {
	index := bytes.Index(_allowedStatusChars, value)
	if index == -1 {
		return ErrInvalidObjectStatusX
	}
	return nil
}

// checkStatusX asserts that a byte slice is a valid possibility (" MTADRCU?!").
func checkStatusY(value []byte) error {
	index := bytes.Index(_allowedStatusChars, value)
	if index == -1 {
		return ErrInvalidObjectStatusY
	}
	return nil
}

// checkPath asserts that a byte slice is non-empty.
func checkPath(value []byte) error {
	// Exists at all.
	if len(value) == 0 {
		return ErrInvalidPath
	}
	return nil
}
