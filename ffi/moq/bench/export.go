// bench/export.c's frame writes and reads through the generated Go, so the two differ by
// what cgo and the Go wrapper add. Each round runs twice, the first to warm up.
package main

import (
	"context"
	"fmt"
	"time"

	"moqtest/moq"
)

const (
	groups = 10000
	frames = 100
)

// perFrame is the nanoseconds per frame since start.
func perFrame(start time.Time) float64 {
	return float64(time.Since(start).Nanoseconds()) / (groups * frames)
}

// must is v, failing the bench on err.
func must[T any](v T, err error) T {
	if err != nil {
		panic(err)
	}
	return v
}

// check fails the bench on err.
func check(err error) {
	if err != nil {
		panic(err)
	}
}

func round() string {
	broadcast := moq.NewBroadcastInfo().Produce()
	micro := moq.TimescaleMICRO()
	track := must(broadcast.CreateTrack("bench", moq.TrackInfoDefault().WithTimescale(micro)))
	micro.Close()

	// A frame of 100 zero bytes, as bench/export.c writes.
	frame := make([]byte, 100)
	var micros uint64

	// Each frame stamped through a fresh timestamp handle, as a caller holding a u64 must.
	start := time.Now()
	for range groups {
		group := must(track.AppendGroup())
		for range frames {
			at := must(moq.TimestampFromMicros(micros))
			micros++
			check(group.WriteFrame(at, frame))
			at.Close()
		}
		check(group.Finish())
		group.Close()
	}
	stamped := perFrame(start)

	// The same, reusing one timestamp handle: what the handle costs.
	at := must(moq.TimestampFromMicros(micros))
	start = time.Now()
	for range groups {
		group := must(track.AppendGroup())
		for range frames {
			check(group.WriteFrame(at, frame))
		}
		check(group.Finish())
		group.Close()
	}
	reused := perFrame(start)

	// The floor: a hold of a handle and a bool back.
	closed := 0
	start = time.Now()
	for range groups * frames {
		if track.IsClosed() {
			closed++
		}
	}
	call := perFrame(start)

	// Reads: each frame written, then read through a call whose future is ready at its
	// first poll, so it never waits.
	ctx := context.Background()
	consumer := track.Consume()
	subscriber := must(consumer.Subscribe(ctx, nil))
	consumer.Close()
	var last *moq.FrameFrame
	start = time.Now()
	for g := range groups {
		group := must(track.AppendGroup())
		reader := must(subscriber.RecvGroup(ctx))
		for f := range frames {
			check(group.WriteFrame(at, frame))
			read := must(reader.ReadFrame(ctx))
			if g == groups-1 && f == frames-1 {
				last = read
			} else {
				read.Close()
			}
		}
		reader.Close()
		check(group.Finish())
		group.Close()
	}
	read := perFrame(start)
	subscriber.Close()

	// Payload reads: the last frame's 100 bytes, copied into Go, and Rust's freed.
	start = time.Now()
	for range groups * frames {
		closed += int(last.Payload()[0])
	}
	payload := perFrame(start)
	last.Close()
	at.Close()
	track.Close()
	broadcast.Close()

	bang := ""
	if closed != 0 {
		bang = "!"
	}
	return fmt.Sprintf("export (Go): %.0f ns per stamped 100 B frame, %.0f reusing the timestamp, %.1f ns per handle call, %.0f ns per frame written and read, %.0f ns per payload read%s",
		stamped, reused, call, read, payload, bang)
}

func main() {
	round()
	fmt.Println(round())
}
