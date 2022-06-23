package gitoutput

import (
	"testing"
)

func Test_checkValid(t *testing.T) {
	type args struct {
		fieldType Field
		value     []byte
	}
	tests := []struct {
		name    string
		args    args
		wantErr bool
	}{
		{
			name: "ObjectMode",
			args: args{
				fieldType: ObjectMode,
				value:     []byte("100644"),
			},
			wantErr: false,
		},
		{
			name: "ObjectType",
			args: args{
				fieldType: ObjectType,
				value:     []byte("blob"),
			},
			wantErr: false,
		},
		{
			name: "ObjectName",
			args: args{
				fieldType: ObjectName,
				value:     []byte("8992ebf37df05fc5ff64c0f811a3259adff10d70"),
			},
			wantErr: false,
		},
		{
			name: "ObjectStage",
			args: args{
				fieldType: ObjectStage,
				value:     []byte("0"),
			},
			wantErr: false,
		},
		{
			name: "StatusX",
			args: args{
				fieldType: StatusX,
				value:     []byte("!"),
			},
			wantErr: false,
		},
		{
			name: "StatusY",
			args: args{
				fieldType: StatusY,
				value:     []byte("?"),
			},
			wantErr: false,
		},
		{
			name: "Path",
			args: args{
				fieldType: Path,
				value:     []byte("/hello/world"),
			},
			wantErr: false,
		},
		{
			name: "Unknown",
			args: args{
				fieldType: Field(12),
				value:     []byte("unused"),
			},
			wantErr: true,
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if err := checkValid(tt.args.fieldType, tt.args.value); (err != nil) != tt.wantErr {
				t.Errorf("checkValid() error = %v, wantErr %v", err, tt.wantErr)
			}
		})
	}
}

func Test_checkObjectMode(t *testing.T) {
	type args struct {
		value []byte
	}
	tests := []struct {
		name    string
		args    args
		wantErr bool
	}{
		{
			name: "Simple",
			args: args{
				value: []byte("100644"),
			},
			wantErr: false,
		},
		{
			name: "All sevens",
			args: args{
				value: []byte("777777"),
			},
			wantErr: false,
		},
		{
			name: "All zeroes",
			args: args{
				value: []byte("000000"),
			},
			wantErr: false,
		},
		{
			name: "Non-octal chars",
			args: args{
				value: []byte("sixsix"),
			},
			wantErr: true,
		},
		{
			name: "nul",
			args: args{
				value: []byte("\000\000\000\000\000\000"),
			},
			wantErr: true,
		},
		{
			name: "too long",
			args: args{
				value: []byte("1234567"),
			},
			wantErr: true,
		},
		{
			name: "off by plus one",
			args: args{
				value: []byte("888888"),
			},
			wantErr: true,
		},
		{
			name: "off by minus one",
			args: args{
				value: []byte("//////"),
			},
			wantErr: true,
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if err := checkObjectMode(tt.args.value); (err != nil) != tt.wantErr {
				t.Errorf("checkObjectMode() error = %v, wantErr %v", err, tt.wantErr)
			}
		})
	}
}

func Test_checkObjectType(t *testing.T) {
	type args struct {
		value []byte
	}
	tests := []struct {
		name    string
		args    args
		wantErr bool
	}{
		{
			name: "Finds blob",
			args: args{
				value: []byte("blob"),
			},
			wantErr: false,
		},
		{
			name: "Finds tree",
			args: args{
				value: []byte("tree"),
			},
			wantErr: false,
		},
		{
			name: "Finds commit",
			args: args{
				value: []byte("commit"),
			},
			wantErr: false,
		},
		{
			name: "nonsense input",
			args: args{
				value: []byte("input"),
			},
			wantErr: true,
		},
		{
			name: "Knows too much about the implementation details (all 3)",
			args: args{
				value: []byte("blob tree commit"),
			},
			wantErr: true,
		},
		{
			name: "Knows too much about the implementation details (first two)",
			args: args{
				value: []byte("blob tree"),
			},
			wantErr: true,
		},
		{
			name: "Knows too much about the implementation details (last two)",
			args: args{
				value: []byte("tree commit"),
			},
			wantErr: true,
		},
		{
			name: "Knows too much about the implementation details (arbitrary substring)",
			args: args{
				value: []byte("tree c"),
			},
			wantErr: true,
		},
		{
			name: "Knows too much about the implementation details (space)",
			args: args{
				value: []byte(" "),
			},
			wantErr: true,
		},
		{
			name: "Knows too much about the implementation details (empty string)",
			args: args{
				value: []byte(""),
			},
			wantErr: true,
		},
		{
			name: "Knows too much about the implementation details (leading space)",
			args: args{
				value: []byte(" tree"),
			},
			wantErr: true,
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if err := checkObjectType(tt.args.value); (err != nil) != tt.wantErr {
				t.Errorf("checkObjectType() error = %v, wantErr %v", err, tt.wantErr)
			}
		})
	}
}

func TestCheckObjectName(t *testing.T) {
	type args struct {
		value []byte
	}
	tests := []struct {
		name    string
		args    args
		wantErr bool
	}{
		{
			name: "Simple",
			args: args{
				value: []byte("8992ebf37df05fc5ff64c0f811a3259adff10d70"),
			},
			wantErr: false,
		},
		{
			name: "Too short",
			args: args{
				value: []byte("8992ebf37df05fc5ff64"),
			},
			wantErr: true,
		},
		{
			name: "Too long",
			args: args{
				value: []byte("8992ebf37df05fc5ff64c0f811a3259adff10d708992ebf37df05fc5ff64c0f811a3259adff10d70"),
			},
			wantErr: true,
		},
		{
			name: "Not hex",
			args: args{
				value: []byte("z992ebf37df05fc5ff64c0f811a3259adff10d70"),
			},
			wantErr: true,
		},
		{
			name: "Not lowercase",
			args: args{
				value: []byte("8992EBF37DF05FC5FF64C0F811A3259ADFF10D70"),
			},
			wantErr: true,
		},
		{
			name: "Off by plus one in the ASCII table (a-f).",
			args: args{
				value: []byte("gggggggggggggggggggggggggggggggggggggggg"),
			},
			wantErr: true,
		},
		{
			name: "Off by minus one in the ASCII table (a-f).",
			args: args{
				value: []byte("````````````````````````````````````````"),
			},
			wantErr: true,
		},
		{
			name: "Off by minus one in the ASCII table (0-9).",
			args: args{
				value: []byte("////////////////////////////////////////"),
			},
			wantErr: true,
		},
		{
			name: "Off by plus one in the ASCII table (0-9).",
			args: args{
				value: []byte("::::::::::::::::::::::::::::::::::::::::"),
			},
			wantErr: true,
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if err := CheckObjectName(tt.args.value); (err != nil) != tt.wantErr {
				t.Errorf("CheckObjectName() error = %v, wantErr %v", err, tt.wantErr)
			}
		})
	}
}

func Test_checkObjectStage(t *testing.T) {
	type args struct {
		value []byte
	}
	tests := []struct {
		name    string
		args    args
		wantErr bool
	}{
		{
			name: "0",
			args: args{
				value: []byte("0"),
			},
			wantErr: false,
		},
		{
			name: "1",
			args: args{
				value: []byte("1"),
			},
			wantErr: false,
		},
		{
			name: "2",
			args: args{
				value: []byte("2"),
			},
			wantErr: false,
		},
		{
			name: "3",
			args: args{
				value: []byte("3"),
			},
			wantErr: false,
		},
		{
			name: "/",
			args: args{
				value: []byte("/"),
			},
			wantErr: true,
		},
		{
			name: "4",
			args: args{
				value: []byte("4"),
			},
			wantErr: true,
		},
		{
			name: "00",
			args: args{
				value: []byte("00"),
			},
			wantErr: true,
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if err := checkObjectStage(tt.args.value); (err != nil) != tt.wantErr {
				t.Errorf("checkObjectStage() error = %v, wantErr %v", err, tt.wantErr)
			}
		})
	}
}

func Test_checkStatus(t *testing.T) {
	type args struct {
		value []byte
	}
	tests := []struct {
		name    string
		args    args
		wantErr bool
	}{
		{
			name: "Simple",
			args: args{
				value: []byte("D"),
			},
			wantErr: false,
		},
		{
			name: "Space",
			args: args{
				value: []byte(" "),
			},
			wantErr: false,
		},
		{
			name: "Empty",
			args: args{
				value: []byte(""),
			},
			wantErr: true,
		},
		{
			name: "Too long",
			args: args{
				value: []byte("?!"),
			},
			wantErr: true,
		},
		{
			name: "nul",
			args: args{
				value: []byte("\000"),
			},
			wantErr: true,
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if err := checkStatusX(tt.args.value); (err != nil) != tt.wantErr {
				t.Errorf("checkStatusX() error = %v, wantErr %v", err, tt.wantErr)
			}
			if err := checkStatusY(tt.args.value); (err != nil) != tt.wantErr {
				t.Errorf("checkStatusY() error = %v, wantErr %v", err, tt.wantErr)
			}
		})
	}
}

func Test_checkPath(t *testing.T) {
	type args struct {
		value []byte
	}
	tests := []struct {
		name    string
		args    args
		wantErr bool
	}{
		{
			name: "Simple",
			args: args{
				value: []byte("./"),
			},
			wantErr: false,
		},
		{
			name: "newline",
			args: args{
				value: []byte("has\nnewline"),
			},
			wantErr: false,
		},
		{
			name: "Empty",
			args: args{
				value: []byte(""),
			},
			wantErr: true,
		},
		{
			name: "newline",
			args: args{
				value: []byte("\n"),
			},
			wantErr: false,
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if err := checkPath(tt.args.value); (err != nil) != tt.wantErr {
				t.Errorf("checkPath() error = %v, wantErr %v", err, tt.wantErr)
			}
		})
	}
}
