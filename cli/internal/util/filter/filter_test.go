// Copyright (c) 2015-2020 InfluxData Inc. MIT License (MIT)
// https://github.com/influxdata/telegraf
package filter

import (
	"testing"

	"github.com/stretchr/testify/assert"
)

func TestCompile(t *testing.T) {
	f, err := Compile([]string{})
	assert.NoError(t, err)
	assert.Nil(t, f)

	f, err = Compile([]string{"cpu"})
	assert.NoError(t, err)
	assert.True(t, f.Match("cpu"))
	assert.False(t, f.Match("cpu0"))
	assert.False(t, f.Match("mem"))

	f, err = Compile([]string{"cpu*"})
	assert.NoError(t, err)
	assert.True(t, f.Match("cpu"))
	assert.True(t, f.Match("cpu0"))
	assert.False(t, f.Match("mem"))

	f, err = Compile([]string{"cpu", "mem"})
	assert.NoError(t, err)
	assert.True(t, f.Match("cpu"))
	assert.False(t, f.Match("cpu0"))
	assert.True(t, f.Match("mem"))

	f, err = Compile([]string{"cpu", "mem", "net*"})
	assert.NoError(t, err)
	assert.True(t, f.Match("cpu"))
	assert.False(t, f.Match("cpu0"))
	assert.True(t, f.Match("mem"))
	assert.True(t, f.Match("network"))
}

func TestIncludeExclude(t *testing.T) {
	tags := []string{}
	labels := []string{"best", "com_influxdata", "timeseries", "com_influxdata_telegraf", "ever"}

	filter, err := NewIncludeExcludeFilter([]string{}, []string{"com_influx*"})
	if err != nil {
		t.Fatalf("Failed to create include/exclude filter - %v", err)
	}

	for i := range labels {
		if filter.Match(labels[i]) {
			tags = append(tags, labels[i])
		}
	}

	assert.Equal(t, []string{"best", "timeseries", "ever"}, tags)
}

var benchbool bool

func BenchmarkFilterSingleNoGlobFalse(b *testing.B) {
	f, _ := Compile([]string{"cpu"})
	var tmp bool
	for n := 0; n < b.N; n++ {
		tmp = f.Match("network")
	}
	benchbool = tmp
}

func BenchmarkFilterSingleNoGlobTrue(b *testing.B) {
	f, _ := Compile([]string{"cpu"})
	var tmp bool
	for n := 0; n < b.N; n++ {
		tmp = f.Match("cpu")
	}
	benchbool = tmp
}

func BenchmarkFilter(b *testing.B) {
	f, _ := Compile([]string{"cpu", "mem", "net*"})
	var tmp bool
	for n := 0; n < b.N; n++ {
		tmp = f.Match("network")
	}
	benchbool = tmp
}

func BenchmarkFilterNoGlob(b *testing.B) {
	f, _ := Compile([]string{"cpu", "mem", "net"})
	var tmp bool
	for n := 0; n < b.N; n++ {
		tmp = f.Match("net")
	}
	benchbool = tmp
}

func BenchmarkFilter2(b *testing.B) {
	f, _ := Compile([]string{"aa", "bb", "c", "ad", "ar", "at", "aq",
		"aw", "az", "axxx", "ab", "cpu", "mem", "net*"})
	var tmp bool
	for n := 0; n < b.N; n++ {
		tmp = f.Match("network")
	}
	benchbool = tmp
}

func BenchmarkFilter2NoGlob(b *testing.B) {
	f, _ := Compile([]string{"aa", "bb", "c", "ad", "ar", "at", "aq",
		"aw", "az", "axxx", "ab", "cpu", "mem", "net"})
	var tmp bool
	for n := 0; n < b.N; n++ {
		tmp = f.Match("net")
	}
	benchbool = tmp
}
