package gitoutput

import "bytes"

var _allowedObjectType = []byte("blob tree commit ")
var _allowedStatusChars = []byte(" MTADRCU?!")

func checkValid(fieldType field, value *[]byte) error {
	switch fieldType {
	case ObjectMode:
		return checkObjectMode(value)
	case ObjectType:
		return checkObjectType(value)
	case ObjectName:
		return checkObjectName(value)
	case ObjectStage:
		return checkObjectStage(value)
	case StatusX:
		return checkStatusX(value)
	case StatusY:
		return checkStatusY(value)
	case Path:
		return checkPath(value)
	default:
		return ErrFieldCount
	}
}

func checkObjectMode(value *[]byte) error {
	if len(*value) != 6 {
		return ErrInvalidObjectMode
	}

	// 0-7 are 0x30 - 0x37
	for _, currentByte := range *value {
		if (currentByte ^ 0x30) > 7 {
			return ErrInvalidObjectMode
		}
	}

	// length of 6, 0-7
	return nil
}

func checkObjectType(value *[]byte) error {
	// Because of the space separator, there is no way to pass in a space.
	index := bytes.Index(_allowedObjectType, *value)
	if index != -1 && _allowedObjectType[index+len(*value)] != byte(space) {
		return ErrInvalidObjectType
	}
	return nil
}

func checkObjectName(value *[]byte) error {
	if len(*value) != 40 {
		return ErrInvalidObjectName
	}

	// 0-9 are 0x30 - 0x39
	// a-f are 0x61 - 0x66
	for _, currentByte := range *value {
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

func checkObjectStage(value *[]byte) error {
	// 0-3 are 0x30 - 0x33
	if len(*value) != 1 {
		return ErrInvalidObjectStage
	}

	for _, currentByte := range *value {
		if (currentByte ^ 0x30) >= 4 {
			return ErrInvalidObjectStage
		}
	}

	return nil
}

func checkStatusX(value *[]byte) error {
	index := bytes.Index(_allowedStatusChars, *value)
	if index == -1 {
		return ErrInvalidObjectStatusX
	}
	return nil
}

func checkStatusY(value *[]byte) error {
	index := bytes.Index(_allowedStatusChars, *value)
	if index == -1 {
		return ErrInvalidObjectStatusY
	}
	return nil
}

func checkPath(value *[]byte) error {
	// Exists at all.
	if len(*value) == 0 {
		return ErrInvalidPath
	}
	return nil
}
