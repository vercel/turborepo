package gitoutput

import "testing"

func Test_checkValid(t *testing.T) {
	type args struct {
		fieldType field
		value     []byte
	}
	tests := []struct {
		name    string
		args    args
		wantErr bool
	}{
		// TODO: Add test cases.
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if err := checkValid(tt.args.fieldType, tt.args.value); (err != nil) != tt.wantErr {
				t.Errorf("checkValid() error = %v, wantErr %v", err, tt.wantErr)
			}
		})
	}
}
