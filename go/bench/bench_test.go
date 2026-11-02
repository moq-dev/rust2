// Times the codec's workloads, rust2-ts's from ts/bench/workloads.ts, translated to Go
// and through rust2-ffi's C ABI by cgo: `just go bench`.
package bench

import (
	"bytes"
	"encoding/binary"
	"fmt"
	"strings"
	"testing"

	"codec"
)

type workload struct {
	name  string
	items int
	input []uint8
}

func u64(v uint64) []uint8 { return binary.BigEndian.AppendUint64(nil, v) }

// workloads are ts/bench/workloads.ts's that code moq's wire format: each encode, and a
// decode of what it wrote.
func workloads() []workload {
	var out []workload
	pair := func(name string, items int, op, selector uint8, params ...uint64) []uint8 {
		input := []uint8{op, selector}
		for _, p := range params {
			input = append(input, u64(p)...)
		}
		wire := codec.BenchRun(input)
		name = strings.Replace(name, "%", fmt.Sprintf("%dB", len(wire)/items), 1)
		out = append(out, workload{name + "/encode", items, input})
		out = append(out, workload{name + "/decode", items, append([]uint8{op + 1, selector}, wire...)})
		return wire
	}
	const run = 1024
	for _, v := range []uint64{1<<6 - 1, 1<<14 - 1, 1<<30 - 1, 1<<53 - 1} {
		pair("varint/quic/%", run, 0, 0, run, v)
	}
	for _, v := range []uint64{1<<7 - 1, 1<<14 - 1, 1<<28 - 1, 1<<53 - 1} {
		pair("varint/leading-ones/%", run, 0, 1, run, v)
	}
	for _, v := range []struct {
		name     string
		selector uint8
	}{{"lite-06", 5}, {"lite-07", 6}} {
		video := pair("frames/video/"+v.name, 60, 2, v.selector, 60, 33_333, 60_000, 8_000, 10, 17_000)
		input := append(append([]uint8{7, v.selector}, u64(3)...), video...)
		out = append(out, workload{"frames/video/" + v.name + "/3B-reads/decode", 60, input})
		pair("frames/audio/"+v.name, 50, 2, v.selector, 50, 20_000, 160, 160, 0, 0)
		pair("group/"+v.name, 1, 4, v.selector, 3, 1_234)
	}
	return out
}

// TestAgree checks the two routes give the same output on every workload, so the
// benchmark times the same work.
func TestAgree(t *testing.T) {
	if !contract() {
		t.Fatal("the library and codec.h declare different APIs")
	}
	for _, w := range workloads() {
		if got, want := codec.BenchRun(w.input), native(w.input); !bytes.Equal(got, want) {
			t.Errorf("%s: translated %x, cgo %x", w.name, got, want)
		}
	}
}

func BenchmarkCodec(b *testing.B) {
	for _, w := range workloads() {
		for _, route := range []struct {
			name string
			run  func([]uint8) []uint8
		}{{"go", codec.BenchRun}, {"cgo", native}} {
			b.Run(w.name+"/"+route.name, func(b *testing.B) {
				for b.Loop() {
					route.run(w.input)
				}
				b.ReportMetric(float64(b.Elapsed().Nanoseconds())/float64(b.N*w.items), "ns/item")
			})
		}
	}
}
