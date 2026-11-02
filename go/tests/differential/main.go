// Runs a fixture's translated harness on each input from stdin and writes each outcome
// to stdout, for fixtures.rs to compare with native Rust's.
//
// Each input is a little-endian u32 length and that many bytes. Each outcome is a byte,
// 0 for output and 1 for a panic, then a u32 length and the output or panic message.
package main

import (
	"bufio"
	"encoding/binary"
	"io"
	"os"

	fixture "fixture"
)

func run(input []uint8) (out []uint8, panicked bool) {
	defer func() {
		if r := recover(); r != nil {
			p, ok := r.(fixture.RustPanic)
			if !ok {
				panic(r)
			}
			out, panicked = []uint8(p.Message), true
		}
	}()
	return fixture.Run(input), false
}

func main() {
	in := bufio.NewReader(os.Stdin)
	w := bufio.NewWriter(os.Stdout)
	defer w.Flush()
	var n [4]uint8
	for {
		if _, err := io.ReadFull(in, n[:]); err == io.EOF {
			return
		} else if err != nil {
			panic(err)
		}
		input := make([]uint8, binary.LittleEndian.Uint32(n[:]))
		if _, err := io.ReadFull(in, input); err != nil {
			panic(err)
		}
		out, panicked := run(input)
		kind := uint8(0)
		if panicked {
			kind = 1
		}
		w.WriteByte(kind)
		binary.LittleEndian.PutUint32(n[:], uint32(len(out)))
		w.Write(n[:])
		w.Write(out)
	}
}
