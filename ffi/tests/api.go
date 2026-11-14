// The api fixture's Go test: the script fixtures/api/examples/transcript.rs runs natively,
// through the generated Go. tests/fixtures.rs compares the transcripts.
//
// With an argument, it instead checks the futures' semantics, what Rust keeps of closures
// and reads of readers, what closing and the GC free, what a closed object returns, how
// long returned bytes live, or breaks a contract, which must fail.
package main

import (
	"bufio"
	"context"
	"errors"
	"fmt"
	"io"
	"math"
	"math/big"
	"net/netip"
	"os"
	"runtime"
	"strings"
	"sync"
	"sync/atomic"
	"time"
	"unsafe"

	"api/api"
	"api/dep"
	"api/rt"
	"api/serdejson"
	"api/std"
	"api/tls"
	"api/tracingcore"
	"api/url"
)

// printLog keeps even levels.
type printLog struct{}

// Close runs once Rust drops the implementation.
func (printLog) Close() error {
	fmt.Println("freed")
	return nil
}

func (printLog) Log(level uint8, line string) bool {
	fmt.Printf("log %d %s\n", level, line)
	return level%2 == 0
}

// list is items as Rust's Debug prints a slice, each by show.
func list[T any](items []T, show func(T) string) string {
	shown := make([]string, len(items))
	for i, item := range items {
		shown[i] = show(item)
	}
	return "[" + strings.Join(shown, ", ") + "]"
}

func f64Bits(f float64) string {
	return number(math.Float64bits(f))
}

// numbers prints each of n's lists, its floats as their bits, as transcript.rs does.
func numbers(what string, n api.Numbers) {
	fmt.Printf("%s u16 %s\n", what, list(n.U16s, number))
	fmt.Printf("%s u32 %s\n", what, list(n.U32s, number))
	fmt.Printf("%s u64 %s\n", what, list(n.U64s, number))
	fmt.Printf("%s i8 %s\n", what, list(n.I8s, number))
	fmt.Printf("%s i16 %s\n", what, list(n.I16s, number))
	fmt.Printf("%s i32 %s\n", what, list(n.I32s, number))
	fmt.Printf("%s i64 %s\n", what, list(n.I64s, number))
	fmt.Printf("%s f32 %s\n", what, list(n.F32s, func(f float32) string { return number(math.Float32bits(f)) }))
	fmt.Printf("%s f64 %s\n", what, list(n.F64s, f64Bits))
	if n.Some == nil {
		fmt.Printf("%s some None\n", what)
	} else {
		fmt.Printf("%s some Some(%s)\n", what, list(*n.Some, number))
	}
}

func quoted(s string) string {
	return fmt.Sprintf("%q", s)
}

func number[T any](n T) string {
	return fmt.Sprint(n)
}

func bound(b rt.BoundU64) string {
	switch b.Kind {
	case rt.BoundIncluded:
		return fmt.Sprintf("Included(%d)", b.Value)
	case rt.BoundExcluded:
		return fmt.Sprintf("Excluded(%d)", b.Value)
	}
	return "Unbounded"
}

// escaped is a path's bytes as Rust's escape_ascii prints them.
func escaped(path string) string {
	var out strings.Builder
	for _, b := range []byte(path) {
		switch {
		case b == '\t':
			out.WriteString(`\t`)
		case b == '\r':
			out.WriteString(`\r`)
		case b == '\n':
			out.WriteString(`\n`)
		case b == '\\' || b == '\'' || b == '"':
			out.WriteByte('\\')
			out.WriteByte(b)
		case b >= 0x20 && b < 0x7f:
			out.WriteByte(b)
		default:
			fmt.Fprintf(&out, `\x%02x`, b)
		}
	}
	return out.String()
}

func check(ok bool, what string) {
	if !ok {
		fmt.Fprintf(os.Stderr, "failed: %s\n", what)
		os.Exit(1)
	}
}

func must[T any](v T, err error) T {
	if err != nil {
		panic(err)
	}
	return v
}

// ok fails the test on err, of a call that returns nothing else.
func ok(err error) {
	if err != nil {
		panic(err)
	}
}

func ptr[T any](v T) *T {
	return &v
}

// watched is a context that reports when a call first waits on it, which it does only
// once its future pends, and that holds that call between a wake and its next poll until
// the test releases it.
type watched struct {
	context.Context
	waiting chan struct{}
	once    sync.Once
	gate    atomic.Pointer[chan struct{}]
}

func watch(ctx context.Context) *watched {
	return &watched{Context: ctx, waiting: make(chan struct{})}
}

func (w *watched) Done() <-chan struct{} {
	w.once.Do(func() { close(w.waiting) })
	return w.Context.Done()
}

func (w *watched) Err() error {
	if gate := w.gate.Load(); gate != nil {
		<-*gate
	}
	return w.Context.Err()
}

func script() {
	ctx := context.Background()
	fmt.Printf("limit %d\n", api.LIMIT)

	p := api.Point{X: 1, Y: 2}.Offset(2, 3)
	fmt.Printf("offset %d %d\n", p.X, p.Y)
	fmt.Printf("debug %s\n", p.GoString())
	fmt.Printf("eq %t\n", p.Equal(api.Point{X: 3, Y: 5}))
	d := api.PointDefault()
	fmt.Printf("default %d %d\n", d.X, d.Y)

	c := api.NewCounter("c", 5)
	fmt.Printf("get %d\n", must(c.Get()))
	fmt.Printf("name %s\n", must(c.Name()))
	fmt.Printf("add ok %d\n", must(c.Add(10)))
	// An exported error type is returned as itself.
	var e api.Error
	_, err := c.Add(2000)
	check(errors.As(err, &e), "add returns an api.Error")
	fmt.Printf("add err %s %s\n", e.Error(), e.GoString())
	_, err = c.Sub(20)
	check(errors.As(err, &e), "sub returns an api.Error")
	fmt.Printf("sub err %s %s\n", err, e.GoString())
	ok(c.Rename("d"))
	fmt.Printf("name %s\n", must(c.Name()))
	fmt.Printf("is d %t\n", must(c.Is("d")))
	cp := must(c.Clone())
	must(cp.Add(1))
	fmt.Printf("clone %d original %d\n", must(cp.Get()), must(c.Get()))
	cp.Close()
	fmt.Printf("debug %s\n", c.GoString())
	label := must(c.IntoLabel(&p))
	fmt.Printf("label %s\n", label.GoString())
	fmt.Printf("note %t %t\n", api.HasText(api.Note{Text: ptr("")}), api.HasText(api.NoteDefault()))

	below, above, err := api.NewCounter("s", 7).Split(5)
	ok(err)
	fmt.Printf("split %s %d %s %d\n", must(below.Name()), must(below.Get()), must(above.Name()), must(above.Get()))
	one := must(api.Only(api.NewCounter("o", 3)))
	fmt.Printf("only %s %d\n", must(one.Name()), must(one.Get()))
	w8, w16, w32, w64 := api.Widths(98305)
	fmt.Printf("widths %d %d %d %d\n", w8, w16, w32, w64)
	for _, b := range []uint64{2, 0} {
		if q, r, err := api.Divide(7, b); err != nil {
			fmt.Printf("divide err %s\n", err)
		} else {
			fmt.Printf("divide %d %d\n", q, r)
		}
	}
	for _, n := range []uint64{98305, 0} {
		if s := api.Spread(n); s != nil {
			fmt.Printf("spread %d %d %d %d\n", s.V0, s.V1, s.V2, s.V3)
		} else {
			fmt.Println("spread none")
		}
	}
	for _, needle := range []string{"lo", "x"} {
		if found := api.Find(rt.NewBytes([]byte("hello")), rt.NewBytes([]byte(needle))); found != nil {
			fmt.Printf("find %d %d\n", found.V0, found.V1)
		} else {
			fmt.Println("find none")
		}
	}
	rows := api.Rows([]api.Point{{X: 1, Y: 2}, {X: 3, Y: 3}})
	shapes := map[api.Shape]string{api.ShapeCircle: "Circle", api.ShapeSquare: "Square"}
	fmt.Printf("rows %s\n", strings.Trim(list(rows, func(r rt.Tuple3[string, api.Shape, api.Point]) string {
		return fmt.Sprintf("%s %s %d %d", r.V0, shapes[r.V1], r.V2.X, r.V2.Y)
	}), "[]"))
	// Each handle in a tuple moves in, as one taken by value does.
	board := must(api.NewBoard([]rt.Tuple2[string, *api.Counter]{
		{V0: "low", V1: api.NewCounter("l", 1)},
		{V0: "high", V1: api.NewCounter("h", 9)},
	}))
	fmt.Printf("ranked %s\n", strings.Trim(list(must(board.Ranked()), func(r rt.Tuple2[string, *api.Counter]) string {
		return fmt.Sprintf("%s %s %d", r.V0, must(r.V1.Name()), must(r.V1.Get()))
	}), "[]"))
	best := must(board.Best())
	fmt.Printf("best %s %d\n", must(best.Name()), must(best.Get()))
	best.Close()
	board.Close()
	// A map crosses as its pairs and a set as its items, which Rust collects in order, each
	// key once, the last kept.
	atlas := api.Atlas{
		Points: []rt.Tuple2[string, api.Point]{
			{V0: "b", V1: api.Point{X: 1, Y: 1}},
			{V0: "ab", V1: api.Point{X: 2, Y: 3}},
			{V0: "b", V1: api.Point{X: 4, Y: 4}},
		},
		Ids: []uint32{3, 1, 3},
	}.With("c", api.Point{X: 5, Y: 6})
	atlasPoints := strings.Trim(list(atlas.Points, func(r rt.Tuple2[string, api.Point]) string {
		return fmt.Sprintf("%s %d %d", r.V0, r.V1.X, r.V1.Y)
	}), "[]")
	atlasIds := strings.Trim(fmt.Sprint(atlas.Ids), "[]")
	fmt.Printf("atlas %s; ids %s\n", atlasPoints, atlasIds)
	// Another crate's error crosses as its message.
	for _, text := range []string{"80", "x"} {
		if port, err := api.Port(text); err == nil {
			fmt.Printf("port %d\n", port)
		} else {
			fmt.Printf("port err %s\n", err)
		}
	}
	square := api.ShapeSquare
	span := api.Span{Bounds: rt.Tuple2[uint32, uint32]{V0: 2, V1: 10}, Named: &rt.Tuple2[string, api.Shape]{V0: "s", V1: square}}
	lower, upper := api.Halves(span)
	fmt.Printf("halves %d %d %d %d %s\n", lower.V0, lower.V1, upper.V0, upper.V1, span.GoString())
	n, text := api.Swapped(rt.Tuple2[string, uint32]{V0: "t", V1: 4})
	fmt.Printf("swapped %d %s\n", n, text)
	// A tuple of one is its element, so `Option<(Option<u32>,)>` nests two pointers.
	for _, value := range []**uint32{nil, ptr[*uint32](nil), ptr(ptr[uint32](3))} {
		switch doubled := api.Doubled(value); {
		case doubled == nil:
			fmt.Println("doubled None")
		case *doubled == nil:
			fmt.Println("doubled Some((None,))")
		default:
			fmt.Printf("doubled Some((Some(%d),))\n", **doubled)
		}
	}
	fmt.Printf("lent back %s boxed %s\n", list(api.LentBack(rt.NewBytes([]byte{1, 2})).Copy(), number), list(api.BoxedTuple(rt.NewBytes([]byte{3})).Copy(), number))
	// A borrowed string and an owned one share a tuple struct.
	check(api.Title(false) == nil, "an informal title is none")
	fmt.Printf("title Some((%s,)) None named Some((%s,))\n", quoted(*api.Title(true)), quoted(*api.Named("x")))

	// Each lent counter is a new handle to a clone.
	registry := must(api.NewRegistry(api.NewCounter("first", 1)))
	fmt.Printf("last %t\n", must(registry.Last()) != nil)
	ok(registry.Push(api.NewCounter("last", 2)))
	first := must(registry.FirstGet())
	must(first.Add(10))
	fmt.Printf("first %d lent %d\n", must(must(registry.FirstGet()).Get()), must(first.Get()))
	last := must(registry.Last())
	must(last.Add(10))
	fmt.Printf("last %d lent %d\n", must(must(registry.Last()).Get()), must(last.Get()))
	// A closure lent for the call edits the first counter through a handle.
	by := uint64(5)
	edited := must(registry.Edit(func(counter *api.Counter) bool {
		must(counter.Add(by))
		return must(counter.Edited()) && must(counter.Get()) == 6
	}))
	fmt.Printf("edit %t %d\n", edited, must(must(registry.FirstGet()).Get()))
	// A setter moves an object in, as assignment does, closing its handle.
	ok(registry.FirstSet(api.NewCounter("second", 3)))
	fmt.Printf("set first %d %s\n", must(must(registry.FirstGet()).Get()), must(must(registry.FirstGet()).Name()))
	fmt.Printf("walk %d\n", api.Walk(3, func(cursor *api.Cursor) uint64 { return must(cursor.At()) * 2 }))
	fmt.Printf("each %d\n", api.Each(2, func(at uint32) { fmt.Printf("visit %d\n", at) }))
	fmt.Printf("offset %d\n", api.Offset(func() uint32 { return 7 }, 3))
	fmt.Printf("shift %d\n", api.Shift(func() uint32 { return 7 }, 4))

	name := api.NameOwned("a")
	joined := must(name.Join(api.NameOwned("b")))
	fmt.Printf("joined %s %s %t\n", must(joined.AsStr()), joined.GoString(), must(joined.Equal(name)))
	ok(joined.Rename("c"))
	fmt.Printf("renamed %s\n", must(joined.AsStr()))
	long := must(api.Longer(joined, api.NameOwned("dd")))
	fmt.Printf("longer %s\n", must(long.AsStr()))
	fmt.Printf("unnamed [%s]\n", must(api.Unnamed().AsStr()))
	lit := api.NameBorrowed("lit")
	fmt.Printf("borrowed %s %t %t\n", must(lit.AsStr()), must(lit.Shares(api.NameBorrowed("lit"))), must(lit.Shares(api.NameBorrowed("other"))))

	fmt.Printf("total %d\n", api.Total(rt.RangeU64{Start: 2, End: 5}))
	fmt.Printf("reach %d\n", api.Reach(2, 5))
	bigOne := big.NewInt(1)
	pow := func(n uint) *big.Int { return new(big.Int).Lsh(bigOne, n) }
	fmt.Print("scaled")
	for _, v := range []*big.Int{
		api.Scaled(new(big.Int).Sub(pow(128), bigOne), 1),
		api.Scaled(new(big.Int).Add(pow(64), bigOne), 1<<63),
		api.Scaled(pow(127), 2),
	} {
		if v == nil {
			fmt.Print(" none")
		} else {
			fmt.Printf(" %032x", v)
		}
	}
	fmt.Println()
	{
		digest := api.NewDigest()
		if err := digest.SumSet(must(api.Fingerprint("deadbeef"))); err != nil {
			panic(err)
		}
		_, unparsed := api.Fingerprint("zz")
		fmt.Printf("digest %x %x %s\n", must(digest.SumGet()), must(digest.Flipped([4]byte{0xff, 0, 0xff, 0})), unparsed)
		digest.Close()
	}
	batch, batch2 := api.NewBatch(), api.Pair()
	accepted, paired := 0, 0
	for v := uint32(1); v <= 5; v++ {
		if must(batch.Push(v)) {
			accepted++
		}
	}
	for v := uint32(1); v <= 3; v++ {
		if must(batch2.Push(v)) {
			paired++
		}
	}
	emptied := must(api.Drain4(batch))
	fmt.Printf("batch %d %d %d %d %d %d %d\n", must(batch.Capacity()), accepted, emptied, must(api.Drain4(batch)),
		must(batch2.Capacity()), paired, must(api.Drain2(batch2)))
	fmt.Printf("capacities %d %d\n", must(api.Capacities42(batch, batch2)), must(api.Capacities22(batch2, batch2)))
	wide := api.Widen(rt.RangeU64{Start: 2, End: 5}, 1)
	fmt.Printf("widen %d %d\n", wide.Start, wide.End)
	included := func(n uint64) rt.BoundU64 { return rt.BoundU64{Kind: rt.BoundIncluded, Value: n} }
	excluded := func(n uint64) rt.BoundU64 { return rt.BoundU64{Kind: rt.BoundExcluded, Value: n} }
	unbounded := rt.BoundU64{Kind: rt.BoundUnbounded}
	counts := list([]rt.BoundsU64{
		{Start: included(2), End: excluded(5)},
		{Start: included(2), End: included(5)},
		{Start: included(3), End: unbounded},
		{Start: unbounded, End: unbounded},
		{Start: excluded(2), End: included(4)},
	}, func(b rt.BoundsU64) string { return fmt.Sprint(api.Count(b)) })
	fmt.Printf("count %s\n", strings.ReplaceAll(strings.Trim(counts, "[]"), ",", ""))
	fmt.Printf("end %s %s\n", bound(api.End(3)), bound(api.End(0)))
	start, end := api.Ends(3)
	fmt.Printf("ends %s %s\n", bound(start), bound(end))
	for _, n := range []uint64{2, 5} {
		got, err := api.Below(n, 3)
		var bounds rt.BoundsU64Error
		switch {
		case err == nil:
			fmt.Printf("below %d\n", got)
		case errors.As(err, &bounds):
			fmt.Printf("below err %s %s\n", bound(bounds.Value.Start), bound(bounds.Value.End))
		default:
			check(false, "below returns its bounds")
		}
	}
	window := api.Window{Range: rt.RangeU64{Start: 2, End: 8}, End: api.End(5)}
	fmt.Printf("window %t %t %s\n", window.Contains(4), window.Contains(6), window.GoString())

	fmt.Println(api.Greet("x", api.ShapeSquare))
	fmt.Printf("len %d\n", api.Len(rt.NewBytes([]byte("abc"))))
	binary := []byte{0xff, 0, 0x61}
	fmt.Printf("hex %s\n", api.Hex(rt.NewBytes(binary)))
	fmt.Printf("timed hex %s\n", api.TimedHex(rt.NewBytes(binary)))
	fmt.Printf("sample hex %s\n", api.SampleHex(rt.NewBytes(binary)))
	fmt.Printf("queued hex %s\n", api.QueuedHex(rt.NewBytes(binary)))
	fmt.Printf("tick %s\n", api.TickName(api.Label{Name: "tick"}))
	fmt.Printf("rooted %s\n", api.RootedName(api.Label{Name: "rooted"}))
	fmt.Printf("stored %s\n", api.StoredText("stored"))
	fmt.Printf("layer %s\n", api.LayerName(api.Label{Name: "layer"}))
	for _, event := range []api.Event{api.EventMoved{V0: p}, api.EventNamed{Name: "n"}, api.EventDone{}} {
		fmt.Println(api.Describe(event))
	}

	// A `Copy` value passed by value is copied, so it stays usable.
	stamp := api.StampFromMillis(2500)
	fmt.Printf("secs %d\n", stamp.Secs())
	fmt.Printf("later %t\n", api.StampFromMillis(3000).Later(stamp))
	fmt.Printf("secs %d\n", stamp.Secs())
	fmt.Printf("whole %s\n", stamp.Whole().GoString())
	message := api.NewMessage(&stamp, rt.NewBytes([]byte("body")))
	fmt.Printf("payload %s\n", must(message.PayloadGet()).Copy())
	fmt.Printf("stamp %s\n", must(message.StampGet()).GoString())
	fmt.Printf("sent %t\n", must(message.Sent()))
	// Setters assign in place: the bytes are copied in, and a `Copy` value copied.
	reset := api.StampFromMillis(4000)
	ok(message.StampSet(&reset))
	ok(message.PayloadSet(rt.NewBytes([]byte("edited"))))
	fmt.Printf("payload %s\n", must(message.PayloadGet()).Copy())
	fmt.Printf("stamp %s\n", must(message.StampGet()).GoString())
	fmt.Printf("secs %d\n", reset.Secs())
	ok(message.StampSet(nil))
	fmt.Printf("stamp %t\n", must(message.StampGet()) != nil)

	// A value past a word, copied across: a closure takes and returns one by value.
	moment := *api.NewMoment(1500, 1000)
	fmt.Printf("moment %d %t\n", moment.Millis(), api.NewMoment(1, 0) == nil)
	shifted := api.Shifted(moment, func(m api.Moment) api.Moment { return m.After(500) })
	fmt.Printf("shifted %d %t\n", shifted.Millis(), shifted.Equal(moment.After(500)))
	fmt.Printf("wrapped %d %t\n", moment.After(math.MaxUint64).Millis(), moment.Equal(shifted))

	// A padded value crosses with its padding zeroed, and one whose bytes vary is a handle.
	gap := api.NewGap(7, 3)
	fmt.Printf("gap %d %t\n", gap.Sum(), gap.Equal(api.NewGap(7, 3)))
	// Its `u64` then its `u8`, so every byte past the ninth is padding Rust made `0xff`.
	padding := unsafe.Slice((*byte)(unsafe.Pointer(&gap.Bytes_)), unsafe.Sizeof(gap.Bytes_))[9:]
	for n, b := range padding {
		if b != 0 {
			panic(fmt.Sprintf("gap's padding byte %d is not zero", n+9))
		}
	}
	four := uint32(4)
	berth := api.NewBerth(&four)
	empty := api.NewBerth(nil)
	berthAt := *must(berth.At())
	emptyAt := "Some"
	if must(empty.At()) == nil {
		emptyAt = "None"
	}
	fmt.Printf("berth Some(%d) %s\n", berthAt, emptyAt)
	berth.Close()
	empty.Close()

	// A `Copy` type changed in place is a value, whose change reaches only the copy changed.
	score := api.ScoreOf(2)
	step := api.ScoreOf(3)
	score.Add(step)
	score.Add(step)
	fmt.Printf("score %d %d %d\n", score.Count(), step.Doubled().Count(), step.Count())
	scoreCopy := score
	taken := scoreCopy.Take()
	fmt.Printf("taken %d %d %d\n", taken, scoreCopy.Count(), score.Count())
	otherScore := step
	before0, before1 := scoreCopy.Absorb(&otherScore)
	fmt.Printf("absorbed %d %d %d %d %d\n", before0, before1, scoreCopy.Count(), otherScore.Count(), step.Count())
	hits := api.HitsOf(1, 2)
	keptHits := hits
	hits.Add(api.HitsOf(10, 20))
	hits.HitsSet(7)
	fmt.Printf("hits %d %d %d %d %t\n", hits.HitsGet(), hits.MissesGet(), keptHits.HitsGet(), keptHits.MissesGet(), hits.Equal(keptHits))
	counted := keptHits
	countedSame := counted.Eq(keptHits)
	fmt.Printf("counted %t %d\n", countedSame, counted.HitsGet())
	extent := api.NewExtent(5, 3)
	extentStart := extent
	old := extent.Slide(10)
	extent.StartSet(extent.StartGet() + 1)
	fmt.Printf("extent %d %d %d %d %d\n", old, extent.StartGet(), extent.End(), extentStart.StartGet(), extentStart.End())
	weighted := api.Weighted{Score: step, Weight: 4}
	fmt.Printf("weighted %d %d\n", weighted.Total(), weighted.Score.Count()+step.Count())

	// Rust frees the implementation once it drops it, releasing Go's.
	fmt.Printf("kept %d\n", api.Emit(printLog{}, rt.NewBytes([]byte{1, 2, 3, 4})))
	logger := api.NewLogger(printLog{})
	fmt.Printf("write %t\n", must(logger.Write("hi")))
	logger.Close()
	fmt.Println("dropped")
	// Rust calls it on another thread, before the call returns.
	threshold := uint8(2)
	filtered := api.Filter(rt.NewBytes([]byte{1, 2, 3, 4}), func(level uint8, line string) bool {
		fmt.Printf("filter %d %s\n", level, line)
		return level > threshold
	})
	fmt.Printf("filtered %d\n", filtered)

	channel := api.NewChannel()
	// The receive waits once its future pends, until the send wakes it.
	watching := watch(ctx)
	got := make(chan []byte)
	go func() { got <- must(channel.Recv(watching)).Copy() }()
	<-watching.waiting
	fmt.Println("recv pending true")
	channel.Send(rt.NewBytes([]byte("hi")))
	payload := <-got
	wakes := 1
	fmt.Printf("woken %d\n", wakes)
	fmt.Printf("recv %s\n", payload)
	channel.Send(rt.Bytes{})
	if _, err := channel.RecvNonempty(ctx); err != nil {
		fmt.Printf("nonempty err %s\n", err)
	}
	fmt.Print("fill")
	for _, v := range append(must(api.Fill4(ctx, api.NewBatch(), 9)), must(api.Fill2(ctx, api.Pair(), 1))...) {
		fmt.Printf(" %d", v)
	}
	fmt.Println()
	// The send happens at the call: the future is cancelled before its first poll.
	cancelled, cancel := context.WithCancel(ctx)
	cancel()
	check(errors.Is(channel.SendLater(cancelled, rt.NewBytes([]byte("eager"))), context.Canceled), "a cancelled call returns its context's error")
	fmt.Printf("recv %s\n", must(channel.Recv(ctx)).Copy())
	// The send happens at the call, and the future borrows the channel and the tag.
	fmt.Printf("tagged %s\n", must(channel.Tagged(ctx, "tag")))
	fmt.Printf("itemized %d\n", must(channel.Itemized(ctx, &[]uint32{1, 2, 3})))
	fmt.Printf("byte_len %d\n", must(api.ByteLen(ctx, rt.NewBytes([]byte("abc")))))
	// An eager call whose callback cancels its context while it makes its future is never
	// polled: only the eager half logs.
	{
		inner, cancel := context.WithCancel(ctx)
		log := &cancelLog{cancel: cancel}
		_, err := api.EmitLater(inner, log, rt.NewBytes([]byte{1, 2}))
		check(errors.Is(err, context.Canceled), "an eager call cancelled as it makes its future returns its context's error")
		check(log.lines.Load() == 2, "an eager call cancelled as it makes its future is never polled")
	}
	channel.Send(rt.NewBytes([]byte("sized")))
	sized, size, err := channel.RecvSized(ctx)
	check(err == nil, "recv sized")
	fmt.Printf("sized %s %d\n", sized.Copy(), size)
	// The waker moves into Rust, which keeps it until the send wakes it.
	channel.Notify(func() { wakes++ })
	channel.Send(rt.NewBytes([]byte("rung")))
	fmt.Printf("woken %d\n", wakes)
	// A poll with no async twin is a future.
	fmt.Printf("take %s\n", must(channel.PollTake(ctx)).Copy())
	channel.Send(rt.NewBytes([]byte("measured")))
	fmt.Printf("measure %d\n", must(channel.PollMeasure(ctx, 2)))
	// A context is made of the waker, which Rust keeps until the send wakes it.
	channel.Listen(func() { wakes++ })
	channel.Send(rt.NewBytes([]byte("heard")))
	fmt.Printf("woken %d\n", wakes)
	// A returned waker wakes the waiter's task through Rust until the GC collects it.
	waiter := api.NewWaiter(func() { wakes++ })
	must(waiter.Waker())()
	must(waiter.Context())()
	waiter.Close()
	fmt.Printf("woken %d\n", wakes)

	list1 := must(api.NewNode(1, nil))
	ok(list1.Push(2))
	fmt.Printf("node %d\n", must(list1.Sum()))

	tally := api.NewTally()
	must(tally.Bump())
	fmt.Printf("tally %d %t\n", must(tally.Bump()), must(tally.Equal(tally)))
	fmt.Printf("both %d\n", must(api.BumpBoth(tally, tally)))
	fmt.Printf("peek %d\n", must(api.Peek(tally, func(lent *api.Tally) uint32 { return must(lent.Bump()) })))

	area := must(api.Grow(dep.NewArea(dep.Size{Width: 2, Height: 3}), 1))
	areaSize := must(area.Size())
	fmt.Printf("area %d %d %d\n", areaSize.Width, areaSize.Height, must(area.Pixels()))

	// Generic types, each at the instances it infers, named by their arguments.
	pair := api.PairU32{Left: 2, Right: 5}
	swapped := pair.Swap()
	fmt.Printf("pair %d %d sum %d %s %t\n", swapped.Left, swapped.Right, pair.Sum(), pair.GoString(), pair.Equal(swapped))
	fmt.Printf("even %t %t\n", pair.Even(), api.PairU32{Left: 7, Right: 7}.Even())
	// An instance renamed alone, beside another of `Pair` that keeps its name.
	tagged := api.Tag(dep.Size{Width: 1, Height: 2}, 3)
	flipped := tagged.Swap()
	fmt.Printf("sized %d %d %s\n", flipped.Left, flipped.Right.Width, flipped.GoString())
	beside := api.PairU32{Left: 258, Right: 9}.BesideSize(tagged.Left)
	fmt.Printf("beside %d %s\n", beside.Left, beside.GoString())
	// At its default, named as the type itself.
	stack := api.NewStack()
	for i, n := range []string{"a", "b"} {
		ok(stack.Push(api.NewCounter(n, uint64(i)+1)))
	}
	top := must(must(stack.Pop()).Get())
	fmt.Printf("stack %d Some(%d) %t %t\n", must(stack.Depth()), top, must(stack.Pop()) != nil, must(stack.Pop()) == nil)
	// Generic functions and methods, at each implementation of the trait bounding them, by
	// reference, which leaves Go its handle.
	fromPoint := api.CounterSeededPoint("p", api.Point{X: -3, Y: 4})
	fromSize := api.CounterSeededSize("s", dep.Size{Width: 5, Height: 6})
	fromCounter := must(api.CounterSeededCounter("c", fromPoint))
	fromPolar := api.CounterSeededPolarPoint("q", api.PolarPoint{Turns: 1, Distance: 2})
	fmt.Printf("seeded %d %d %d %d past %d seeds %d\n", must(fromPoint.Get()), must(fromSize.Get()), must(fromCounter.Get()), must(fromPolar.Get()),
		must(stack.DepthPastPoint(api.Point{X: 1, Y: 2})), api.SeedsPointSize(dep.Size{Width: 2, Height: 3}))

	// A trait implemented for `Point` and `&Point` apart: one instance.
	fmt.Printf("weigh %d\n", api.WeighPoint(api.Point{X: -1, Y: 2}))
	// A function named as the type it returns, and one named as a type it does not. `area`
	// names a type of another crate's package, which leaves its name alone.
	polar := api.NewPolarPoint(2)
	check(polar != nil, "a point off the origin")
	check(api.Span_(1, 4) == 3, "a function named as a type of its package")
	fmt.Printf("polar %d %t area %d\n", polar.Distance, api.NewPolarPoint(0) == nil, api.Area(dep.Size{Width: 2, Height: 3}))
	// Two types whose paths join alike, the second kept apart where its path splits.
	fmt.Printf("wind %d\n", api.Wind(api.PolarTurn{Whole: 2}, api.Polar_Turn{Part: 3}))
	// Names that clash in some language, each kept apart there.
	clashSpan := api.ClashSpan{Width: 4}
	clashStore := api.NewClashStore(7)
	clashPair := rt.Tuple2[rt.Tuple2[uint8, uint16], uint32]{V0: rt.Tuple2[uint8, uint16]{V0: 1, V1: 2}, V1: 3}
	clashTriple := rt.Tuple3[uint8, uint16, uint32]{V0: 1, V1: 2, V2: 3}
	fmt.Printf("clash %s %s %s span %d %s %s store %d %d audio %d %d nested %d\n",
		api.ClashTone{Hz: 440}.GoString(), api.ClashTone_{Db: 3}.GoString(), api.Cancelled{Step: 2}.GoString(),
		clashSpan.Width_(), clashSpan.ToString(), clashSpan.String(), must(clashStore.Free()), must(clashStore.Close_()),
		api.ClashAudioEncoderFinish(), api.ClashAudioEncoderFinish_(), api.ClashNested(&clashPair, &clashTriple))
	clashStore.Close()
	// Instances that name each other, and one made only by an impl of it.
	crossed := api.Cross(api.LeftRightU8{Value: api.RightU8{Value: 1}}, api.RightLeftU8{Value: api.LeftU8{Value: 2}})
	fmt.Printf("cross %d outer %d\n", crossed, must(api.NewOuterInnerU8(4).Get()))
	// A bound that outlives the call takes a value, and an instance only a constant names.
	fmt.Printf("kept %d boxed %d\n", api.KeptPoint(api.Point{X: 1, Y: -2}), api.BOXED().Value)
	fmt.Printf("keep %d\n", api.KeepPoint(api.Point{X: 1, Y: -2}))
	five := api.Point{X: 5, Y: 2}
	fmt.Printf("kept or %d %d keeper %d\n", api.KeptOrPoint(&five), api.KeptOrPoint(nil), api.KeeperPoint(five))
	// Inferred instances taking a lent closure and a foreign implementation, and one only
	// its impl of a trait names.
	sides := api.PairU32{Left: 2, Right: 5}
	fmt.Printf("apply %d\n", api.ApplyU32(sides, func(side uint32) uint32 { return side * 3 }))
	fmt.Printf("report %d\n", api.ReportU32(sides, printLog{}))
	fmt.Printf("wrapped %d\n", api.KeptWrapU32(api.WrapU32{Value: 4}))
	fmt.Printf("keeps %d\n", api.Keeps(printLog{}).Value)

	// Its Drop runs when Go closes the handle, not before.
	token := api.NewToken(7)
	fmt.Printf("token %d dropped %d\n", must(token.IdGet()), api.Dropped())
	token.Close()
	fmt.Printf("dropped %d\n", api.Dropped())

	// A field and the method of its name, each its own member.
	retry := api.NewRetry(ptr[uint32](5))
	limit := must(retry.LimitGet())
	isSome := 0
	if limit != nil {
		isSome = 1
	}
	fmt.Printf("retry %d %d %d %d\n", isSome, *limit, must(retry.Limit()), must(retry.Delay()))
	// The renamed field's setter, named as its getter.
	ok(retry.LimitSet(nil))
	fmt.Printf("retry %d\n", must(retry.Limit()))
	fmt.Printf("slot %d\n", must(api.NewSlotU8(200).Value()))

	// Foreign types: a URL parsed from text, socket addresses, OS paths, and slices.
	base := must(url.UrlParse("https://example.com/a/b"))
	fmt.Printf("url %s host Some(%s)\n", base, quoted(*must(api.Host(base))))
	resolved := must(api.Resolve(must(base.Clone()), "c?d"))
	fmt.Printf("resolve %s %t\n", resolved, must(resolved.Equal(base)))
	resolved.Close()
	var parse rt.StringError
	_, err = url.UrlParse("no scheme")
	check(errors.As(err, &parse), "parse returns its message")
	fmt.Printf("parse err %s\n", parse.Value)
	v4 := netip.MustParseAddrPort("127.0.0.1:8080")
	v6 := netip.MustParseAddrPort("[::1%3]:443")
	fmt.Printf("ports %s\n", list(api.Ports([]netip.AddrPort{v4, v6}), number))
	peer := api.Peer{Addr: v6, Names: []string{"a", "b"}}.Next("c")
	fmt.Printf("peer %s %s\n", peer.GoString(), list(peer.Aliases(), quoted))
	file := api.File("dir", "f\\'\"\t")
	fmt.Printf("file %s\n", escaped(file))
	fmt.Printf("parents %s\n", strings.Trim(strings.ReplaceAll(list(api.Parents([]string{file, "/"}), func(p *string) string {
		if p == nil {
			return "none"
		}
		return escaped(*p)
	}), ",", ""), "[]"))
	// A path that is no UTF-8 is a Go string all the same.
	check(api.Stray() == "\xff", "a path that is not UTF-8 crosses as its bytes")
	var lent string
	api.VisitStray(func(path string) { lent = path })
	check(lent == "\xff", "a path Rust lends that is not UTF-8 crosses as its bytes")
	// An error Rust returns only boxed is its type.
	_, err = api.Refuse("why")
	var refusal *api.Refusal
	check(errors.As(err, &refusal) && err.Error() == "refused: why", "a boxed error is its type")
	words := api.Words(" one two  three ")
	fmt.Printf("words %d %s\n", len(words), list(words, quoted))
	fmt.Printf("sentence %s %s\n", api.Sentence(&words), api.Sentence(nil))
	fmt.Printf("sentence %s|\n", api.Sentence(&[]string{}))
	abc := "abc"
	fmt.Printf("either %d %d\n", api.Either(&abc), api.Either(nil))
	ab := rt.NewBytes([]byte("ab"))
	fmt.Printf("prefixed %s %s\n", api.Prefixed(rt.NewBytes([]byte("cd")), &ab).Copy(), api.Prefixed(rt.NewBytes([]byte("cd")), nil).Copy())
	fmt.Printf("coordinates %d %d\n", api.Coordinates(&api.Point{X: 2, Y: 3}), api.Coordinates(nil))
	two, three := api.NewCounter("two", 2), api.NewCounter("three", 3)
	fmt.Printf("plus %d %d\n", must(two.Plus(three)), must(two.Plus(nil)))
	fmt.Printf("add_to %t %t %d\n", must(two.AddTo(three)), must(two.AddTo(nil)), must(three.Get()))
	encoded := rt.NewBytes([]byte("0,"))
	api.PutNumber(1, &encoded)
	api.PutNumber(23, &encoded)
	fmt.Printf("put_number %s\n", encoded.Copy())
	entry := rt.Tuple2[rt.Bytes, uint32]{V0: rt.NewBytes([]byte("ab")), V1: 1}
	stamps := api.PushCounted(&entry, 'c')
	fmt.Printf("push_counted %d %s %d\n", stamps, entry.V0.Copy(), entry.V1)
	digits := rt.NewBytes([]byte("7x9"))
	firstDigit := must(api.Digit(&digits))
	_, skipped := api.Digit(&digits)
	lastDigit := must(api.Digit(&digits))
	fmt.Printf("digit %d %t %d %d\n", firstDigit, skipped != nil, lastDigit, digits.Len())
	nudged := api.Point{X: 1, Y: 2}
	api.Nudge(&nudged, 3)
	fmt.Printf("nudge %d %d\n", nudged.X, nudged.Y)
	config := api.Secure("h3")
	fmt.Printf("secure %s %s\n", must(api.AlpnLent(config)), must(api.Alpn(must(config.Clone()))))
	cert := api.Certify(rt.NewBytes([]byte("der")))
	certLen := must(api.CertLen(cert))
	fmt.Printf("cert %d %s %d\n", certLen, must(api.CertDer(cert)).Copy(), must(api.ChainLen([]*tls.Cert{api.Certify(rt.NewBytes([]byte("a"))), api.Certify(rt.NewBytes([]byte("bc")))})))
	session := must(api.Resume("h3", 60))
	fmt.Printf("resume %s %d\n", must(session.Alpn()), must(session.Lifetime()).Seconds)
	session.Close()
	if _, err := api.Resume("", 60); err != nil {
		fmt.Printf("resume err %s\n", err)
	}
	meters := api.NewMeters()
	meter := must(meters.First())
	_ = must(meter.Add(2))
	fmt.Printf("lent %d %d\n", must(meter.Add(3)), must(must(meters.First()).Count()))
	ok(meters.Clear())
	fmt.Printf("cleared %d\n", must(must(meters.First()).Count()))
	lastMeter := must(meters.LastGet())
	_ = must(lastMeter.Add(4))
	fmt.Printf("field lent %d\n", must(lastMeter.Add(1)))
	tallies := api.NewTallies()
	_ = must(must(tallies.First()).Bump())
	fmt.Printf("tallied %d\n", must(must(tallies.First()).Bump()))
	fmt.Printf("chunks %s\n", list(api.Chunks(rt.NewBytes([]byte("abcde")), 2), func(b rt.Bytes) string { return list(b.Copy(), number) }))
	counters := api.Counters([]string{"x", "y"})
	fmt.Printf("counters %s\n", strings.Trim(list(counters, func(c *api.Counter) string {
		return fmt.Sprintf("%s %d", must(c.Name()), must(c.Get()))
	}), "[]"))
	// Each counter moves in, so Go closes it.
	fmt.Printf("worth %d %d\n", must(api.Worth(&[]*api.Counter{counters[0]}, []*api.Counter{counters[1]})), must(api.Worth(nil, nil)))
	held := api.NewCounter("h", 4)
	fmt.Print("held")
	for _, n := range []*uint64{must(api.HeldCount(&held)), must(api.HeldCount(nil))} {
		if n == nil {
			fmt.Print(" None")
		} else {
			fmt.Printf(" Some(%d)", *n)
		}
	}
	fmt.Println()
	b := "b"
	unnamed := "None"
	if s := must(api.BoxedName(nil)); s != nil {
		unnamed = *s
	}
	fmt.Printf("boxed Some(%q) %s %t\n", *must(api.BoxedName(api.BoxedCounter(&b))), unnamed, api.BoxedCounter(nil) == nil)
	fmt.Printf("less %d\n", api.Less(5, 2))
	fmt.Printf("sums %s\n", list(api.Sums([][]uint32{{1, 2}, {}, {3}}), number))
	var leastSets []string
	for _, set := range api.Least([][]uint32{{3, 1}, {}, {2}}) {
		leastSets = append(leastSets, list(set, number))
	}
	fmt.Printf("least %s\n", strings.Join(leastSets, " "))
	numbers("edges", api.NumbersEdges())
	numbers("reversed", api.NumbersEdges().Reversed())
	floatEdges := api.NumbersEdges()
	nan := api.NewMeasure(math.NaN(), false)
	fmt.Printf("float equal %t %t %t %t %t\n", floatEdges.Equal(floatEdges), floatEdges.Equal(api.NumbersEdges()), api.NumbersZero(true).Equal(api.NumbersZero(false)), nan.Equal(nan), api.NewMeasure(math.Copysign(0, -1), true).Equal(api.NewMeasure(0, false)))
	fmt.Printf("sum u32 %d %d\n", api.SumU32(nil), api.SumU32([]uint32{math.MaxUint32, math.MaxUint32, 2}))
	fmt.Printf("doubled i64 %s\n", list(api.DoubledI64([]int64{math.MinInt64, -1, 0, math.MaxInt64}), number))
	floats := api.BoxedFloats([]float64{math.Copysign(0, -1), math.Float64frombits(0x7ff8_0000_0000_0001)})
	fmt.Printf("boxed floats %s\n", list(floats, f64Bits))
	// A block lends a sub-slice in place, and Rust's `None` and empty `Some` stay apart.
	framed := []uint32{9, 1, 2, 9}
	check(api.SumU32(framed[1:3]) == 3, "a sub-slice lends only its own items")
	edges := api.NumbersEdges()
	edges.Some = nil
	check(edges.Reversed().Some == nil, "a nil block in an option is None")
	edges.Some = &[]int32{}
	check(edges.Reversed().Some != nil && len(*edges.Reversed().Some) == 0, "an empty block in an option is Some")
	edges.Some = &[]int32{1, 2}
	check(list(*edges.Reversed().Some, number) == "[2, 1]", "a block in an option crosses whole")
	sum := api.Summed([]api.Point{{X: 1, Y: 2}, {X: -4, Y: 6}})
	none := api.Summed(nil)
	fmt.Printf("summed %d %d %d %d\n", sum.X, sum.Y, none.X, none.Y)
	fmt.Printf("longest %d %d\n", api.Longest([]string{"a", "abc"}), api.Longest(nil))
	noFirst := "None"
	if api.FirstOf(nil) != nil {
		noFirst = "Some"
	}
	fmt.Printf("first of Some(%s) %s\n", quoted(*api.FirstOf([]string{"a", "b"})), noFirst)
	limits := api.ByteLimits(rt.NewBytes([]byte("a")), rt.NewBytes([]byte("bc")))
	breadth := api.Breadth(api.LimitsDuration{Low: 5 * time.Millisecond, High: 2 * time.Second})
	fmt.Printf("limits %s %s breadth %d\n", limits.Low.Copy(), limits.High.Copy(), breadth.Milliseconds())
	parts := []rt.Bytes{rt.NewBytes([]byte("ab")), rt.NewBytes(nil), rt.NewBytes([]byte("c"))}
	fmt.Printf("joined %s %d read %d\n", api.Joined(parts).Copy(), len(api.Joined(nil).Copy()), api.ReadLen(rt.NewBytes([]byte("hello"))))
	doc := must(serdejson.JsonParse(`{"b": [1, 2.5, null], "a": "é"}`))
	fmt.Printf("json %s\n", must(api.Encode(doc)))
	fmt.Printf("timed text %s\n", must(api.TimedText(doc)))
	tagged2 := must(api.Tagged(doc, "t"))
	fmt.Printf("tagged %s\n", must(tagged2.ToText()))
	tagged2.Close()
	// A JSON object's entries cross as pairs of a key and its document, each moved in.
	jsonEntries := must(api.Entries([]rt.Tuple2[string, *serdejson.Json]{
		{V0: "b", V1: must(serdejson.JsonParse(`[1]`))},
		{V0: "a", V1: must(serdejson.JsonParse(`2`))},
	}, "t"))
	fmt.Printf("entries %s\n", strings.Trim(list(jsonEntries, func(r rt.Tuple2[string, *serdejson.Json]) string {
		return fmt.Sprintf("%s %s", r.V0, must(r.V1.ToText()))
	}), "[]"))
	for _, entry := range jsonEntries {
		entry.V1.Close()
	}
	_, err = serdejson.JsonParse(`{"a":`)
	check(errors.As(err, &parse), "parse returns its message")
	fmt.Printf("json parse err %s\n", err)
	site := must(api.NewSite(base))
	fmt.Printf("site %s %s\n", must(site.HomeGet()), must(site.HomeGet()))

	// Slices, passed in and returned as clones, and the socket address of each family.
	fmt.Printf("shapes %s greetings %s\n", list(api.ShapeALL(), func(s api.Shape) string { return shapes[s] }), list(api.GREETINGS(), quoted))
	trail := api.NewTrail([]api.Point{{X: 1, Y: 2}, {X: 3, Y: 4}}, []string{"a", "b"})
	point := func(p api.Point) string { return p.GoString() }
	labeled := func(l api.Label) string { return l.GoString() }
	fmt.Printf("trail %s %s\n", list(must(trail.Points()), point), list(must(trail.Labels()), labeled))
	// Returned iterators, each collected at the call: drained labels are gone after.
	fmt.Printf("iterated %s %s %s\n", list(must(trail.Iter()), point), list(must(trail.Names()), quoted), list(must(trail.Bytes()).Copy(), number))
	fmt.Printf("drained %s then %s\n", list(must(trail.Drain()), labeled), list(must(trail.Labels()), labeled))
	// A filter, collected at the call.
	fmt.Printf("beyond %s\n", strings.Trim(list(must(trail.Beyond(1)), func(p api.Point) string {
		return fmt.Sprintf("%d %d", p.X, p.Y)
	}), "[]"))
	trail.Close()
	fmt.Printf("numbered %s\n", strings.Trim(list(api.Numbered([]string{"p", "q"}), func(c *api.Counter) string {
		return fmt.Sprintf("%s %d", must(c.Name()), must(c.Get()))
	}), "[]"))
	next4, next6 := api.Nexts(v4, v6)
	fmt.Printf("nexts %s %d %d %s\n", next4.Addr(), next4.Port(), next6.Port(), next6.Addr().Zone())

	// serde_json's map iterator, collected as pairs of a key and its document.
	sections := must(api.Sections([]rt.Tuple2[string, *serdejson.Json]{
		{V0: "b", V1: must(serdejson.JsonParse(`[1]`))},
		{V0: "a", V1: must(serdejson.JsonParse(`2`))},
	}))
	// Joined by hand: trimming brackets would eat the last document's own.
	var sectionTexts []string
	for _, section := range sections {
		sectionTexts = append(sectionTexts, fmt.Sprintf("%s %s", section.V0, must(section.V1.ToText())))
		section.V1.Close()
	}
	fmt.Printf("sections %s\n", strings.Join(sectionTexts, ", "))
	// An instant only Rust makes, which each call copies, and compares.
	var began std.Instant = api.Now()
	ended := api.Later(began, 1500*time.Millisecond)
	same := api.Later(began, 0)
	fmt.Printf("instants %d %d %t %t\n", api.Since(ended, began).Milliseconds(), api.Since(began, ended).Milliseconds(),
		began.Equal(same), began.Equal(ended))
	fmt.Printf("has_instant %t %t\n", api.HasInstant(&began), api.HasInstant(nil))
	// tracing's level, and its filter as an optional level whose nil is `OFF`.
	verbosity := api.NewVerbosity(tracingcore.LevelWarn)
	fmt.Printf("verbosity %s %s\n", api.Spell(must(verbosity.Filter())), api.Spell(api.Louder(must(verbosity.Filter()))))
	ok(verbosity.LevelSet(tracingcore.LevelTrace))
	fmt.Printf("verbosity %t %s %s %s\n", must(verbosity.LevelGet()) == tracingcore.LevelTrace, api.Spell(must(verbosity.Filter())),
		api.Spell(api.Louder(must(verbosity.Filter()))), api.Spell(api.Louder(nil)))
	verbosity.Close()

	fmt.Printf("made %s\n", api.Made(3, func(n uint32) string { return fmt.Sprintf("%d times", n) }))
	// A closure taking and returning `Copy` values copies them, never running their `Clone`.
	nine, seven := api.NewTicket(9), api.NewTicket(7)
	renewed := api.Reissue(seven, func(t api.Ticket) api.Ticket { return api.NewTicket(t.Number() + 1) })
	copied := api.Reissue(seven, func(api.Ticket) api.Ticket { return nine })
	fmt.Printf("reissued %d %d, cloned %d\n", renewed.Number(), copied.Number(), api.TicketClones())
	// A kept closure, called on another thread after the call that took it returned.
	spawned := api.NewSpawned(func() uint32 { return 7 })
	fmt.Printf("spawned %d\n", must(spawned.Join()))
	// A kept closure: called again at the instant it asks for, then once its waiter wakes.
	driven := api.Now()
	calls := 0
	stopped := must(api.Drive(context.Background(), driven, func(now std.Instant, waiter *api.Waiter) (*std.Instant, error) {
		calls++
		switch calls {
		case 1:
			later := api.Later(now, 5*time.Millisecond)
			return &later, nil
		case 2:
			fmt.Printf("drive %d\n", api.Since(now, driven).Milliseconds())
			must(waiter.Waker())()
			return nil, nil
		}
		return nil, fmt.Errorf("stopped after %d", calls)
	}))
	fmt.Printf("drive %s\n", stopped)
	tail(strings.NewReader("hello world"))
	tail(brokenReader{})
	// A seek from the start reaches the reader whole, past `math.MaxInt64`, as `Cursor` takes one.
	if at, err := api.SeekTo(farReader{}, math.MaxUint64); err != nil {
		fmt.Printf("sought error %s\n", err)
	} else {
		fmt.Printf("sought %d\n", at)
	}
	fmt.Printf("non zeros %s\n", api.DescribeNonZeros(api.NewNonZeros()))
	hopFirst, hopLast := uint64(1), uint64(math.MaxUint64)
	for _, hop := range []*uint64{nil, &hopFirst, &hopLast} {
		if next := api.NextHop(hop); next != nil {
			fmt.Printf("hop Some(%d)\n", *next)
		} else {
			fmt.Println("hop None")
		}
	}
	fmt.Printf("farther hop %d\n", api.FartherHop(api.PairNonZeroU16{Left: 3, Right: math.MaxUint16}))
}

// farReader is a reader whose reads fail, which seeks anywhere from its start, as Rust's
// Cursor does.
type farReader struct{ brokenReader }

var _ rt.StartSeeker = farReader{}

func (farReader) SeekStart(offset uint64) (uint64, error) {
	return offset, nil
}

// tail prints the last 5 bytes of file, or its error.
func tail(file io.ReadSeeker) {
	if bytes, at, err := api.Tail(file, 5); err != nil {
		fmt.Printf("tail error %s\n", err)
	} else {
		fmt.Printf("tail %s %d\n", bytes.Copy(), at)
	}
}

// brokenReader is a reader whose reads fail, as the transcript's: each seek lands at 0.
type brokenReader struct{}

func (brokenReader) Read([]byte) (int, error) {
	return 0, errors.New("broken")
}

func (brokenReader) Seek(int64, int) (int64, error) {
	return 0, nil
}

// lastReader is bytes in memory whose last read returns err beside its bytes, as a Go
// reader may.
type lastReader struct {
	*strings.Reader
	err error
}

func (r lastReader) Read(p []byte) (int, error) {
	n, err := r.Reader.Read(p)
	if err == nil && r.Reader.Len() == 0 {
		err = r.err
	}
	return n, err
}

// stuckReader reads nothing and no error, forever.
type stuckReader struct {
	brokenReader
}

func (stuckReader) Read([]byte) (int, error) {
	return 0, nil
}

// unseekable is a reader whose seeks fail.
type unseekable struct {
	brokenReader
}

func (unseekable) Seek(int64, int) (int64, error) {
	return 0, errors.New("no seek")
}

// released waits for the GC to collect what released marks, once Rust released the closure
// holding it.
func released(released *atomic.Bool, what string) {
	deadline := time.Now().Add(10 * time.Second)
	for !released.Load() {
		check(time.Now().Before(deadline), what)
		runtime.GC()
		runtime.Gosched()
	}
}

// marked is a value whose collection sets the returned flag, for a closure to hold.
func marked() (*[4]int, *atomic.Bool) {
	var released atomic.Bool
	// Larger than the tiny allocator's blocks, whose objects may never be cleaned up.
	marker := new([4]int)
	runtime.AddCleanup(marker, func(released *atomic.Bool) { released.Store(true) }, &released)
	return marker, &released
}

// closures checks what Rust keeps and releases, and readers' reads, seeks, and errors.
func closures() {
	ctx := context.Background()

	// A kept closure Rust releases once it drops it, after its thread called it.
	{
		marker, freed := marked()
		spawned := api.NewSpawned(func() uint32 { return uint32(marker[0]) + 1 })
		check(must(spawned.Join()) == 1, "a kept closure runs on Rust's thread")
		released(freed, "Rust releases a kept closure it dropped")
	}

	// A kept closure the future holds until it is dropped, unfinished.
	{
		marker, freed := marked()
		called := make(chan struct{}, 1)
		start := api.Now()
		cancelled, cancel := context.WithCancel(ctx)
		done := make(chan error)
		go func() {
			_, err := api.Drive(cancelled, start, func(std.Instant, *api.Waiter) (*std.Instant, error) {
				_ = marker[0]
				select {
				case called <- struct{}{}:
				default:
				}
				return nil, nil
			})
			done <- err
		}()
		<-called
		for range 5 {
			runtime.GC()
			runtime.Gosched()
		}
		check(!freed.Load(), "a pending future keeps its closure")
		cancel()
		check(errors.Is(<-done, context.Canceled), "a cancelled drive returns its context's error")
		released(freed, "a dropped future releases its closure")
	}

	// What a closure returns crosses: the instant it asks for, then its error.
	{
		start := api.Now()
		later := api.Later(start, time.Second)
		var seen []int64
		stopped := must(api.Drive(ctx, start, func(now std.Instant, _ *api.Waiter) (*std.Instant, error) {
			seen = append(seen, api.Since(now, start).Milliseconds())
			if len(seen) == 1 {
				return &later, nil
			}
			return nil, errors.New("")
		}))
		check(stopped == "" && len(seen) == 2 && seen[1] == 1000, "a returned instant and an empty error cross")
	}

	// A read's bytes and error both reach Rust, the error on the next read.
	{
		bytes, at, err := api.Tail(lastReader{strings.NewReader("hello world"), errors.New("late")}, 5)
		check(err != nil && err.Error() == "late" && bytes.Len() == 0 && at == 0, "an error beside bytes is the next read's")
		bytes, at, err = api.Tail(lastReader{strings.NewReader("hello world"), io.EOF}, 5)
		check(err == nil && string(bytes.Copy()) == "world" && at == 6, "io.EOF beside bytes ends the reader")
		_, _, err = api.Tail(stuckReader{}, 5)
		check(err != nil && err.Error() == io.ErrNoProgress.Error(), "a reader reading nothing gives up")
		_, _, err = api.Tail(unseekable{}, 5)
		check(err != nil && err.Error() == "no seek", "a seek's error crosses")
		// A file, as moq-mux's repair reads one.
		file := must(os.CreateTemp("", "rust2-ffi-go-tail"))
		defer os.Remove(file.Name())
		must(file.WriteString("on disk"))
		bytes, at, err = api.Tail(file, 4)
		check(err == nil && string(bytes.Copy()) == "disk" && at == 3, "a file reads as a reader")
		// Lent, never closed: the file is still the caller's.
		check(must(file.Seek(0, io.SeekCurrent)) == 3, "Rust leaves a file it read open, where it sought")
		file.Close()
	}
	fmt.Println("closures ok")
}

// futures checks the futures' semantics beyond the transcript, each by an assertion.
func futures() {
	ctx := context.Background()
	channel := api.NewChannel()

	// The send lands from another goroutine's thread while the receive waits.
	go channel.Send(rt.NewBytes([]byte("blocked")))
	check(string(must(channel.Recv(ctx)).Copy()) == "blocked", "recv returns the payload another goroutine sent")

	// A wake that lands while the future polls, before it returns pending, is kept: the
	// call polls again rather than waiting for a wake that already happened.
	check(must(api.Yielded(ctx, 7)) == 7, "a wake during the poll polls again")

	// A future's output of None is nil.
	half := must(api.Halved(ctx, 4))
	check(half != nil && *half == 2, "a future's Some output reads back")
	check(must(api.Halved(ctx, 3)) == nil, "a future's None output is nil")

	// Cancelled between its wake and its poll, a receive loses nothing: the payload stays
	// for the next.
	{
		inner, cancel := context.WithCancel(ctx)
		watching := watch(inner)
		done := make(chan error)
		go func() {
			_, err := channel.Recv(watching)
			done <- err
		}()
		<-watching.waiting
		// Hold the call between the wake and its next poll, where it reads its context.
		gate := make(chan struct{})
		watching.gate.Store(&gate)
		channel.Send(rt.NewBytes([]byte("kept")))
		cancel()
		close(gate)
		check(errors.Is(<-done, context.Canceled), "a receive cancelled after its wake returns its context's error")
		check(string(must(channel.Recv(ctx)).Copy()) == "kept", "a cancelled receive loses nothing")
	}

	// A context done before the call takes nothing, even a payload ready at the first poll.
	{
		channel.Send(rt.NewBytes([]byte("ready")))
		cancelled, cancel := context.WithCancel(ctx)
		cancel()
		_, err := channel.Recv(cancelled)
		check(errors.Is(err, context.Canceled), "a call with a done context returns its error")
		check(string(must(channel.Recv(ctx)).Copy()) == "ready", "a call with a done context takes nothing")
	}

	// A deadline ends a receive that waits for no wake.
	{
		short, cancel := context.WithTimeout(ctx, time.Millisecond)
		defer cancel()
		_, err := channel.Recv(short)
		check(errors.Is(err, context.DeadlineExceeded), "a receive past its deadline returns its error")
	}

	// A handle closed while a call waits on it stays alive until the call ends.
	{
		other := api.NewChannel()
		watching := watch(ctx)
		got := make(chan string)
		go func() { got <- string(must(other.Recv(watching)).Copy()) }()
		<-watching.waiting
		sender := must(other.Clone())
		other.Close()
		sender.Send(rt.NewBytes([]byte("held")))
		check(<-got == "held", "a closed handle stays alive in the call waiting on it")
		sender.Close()
	}

	// Go functions Rust polls as futures, each run on a goroutine of its own.
	{
		ran := make(chan struct{}, 1)
		check(must(api.After(ctx, func(context.Context) { ran <- struct{}{} }, 4)) == 8 && len(ran) == 1, "Rust awaits a Go function")
	}
	{
		gate := make(chan struct{})
		got := make(chan uint32)
		go func() { got <- must(api.After(ctx, func(context.Context) { <-gate }, 5)) }()
		select {
		case <-got:
			check(false, "Rust waits on a Go function that blocks")
		case <-time.After(10 * time.Millisecond):
		}
		close(gate)
		check(<-got == 10, "a Go function's return wakes Rust")
	}
	{
		err := api.Within(ctx, 0, func(context.Context) {})
		check(errors.Is(err, api.ErrorOverflow{}), "Rust gives up on a Go function")
	}
	{
		short, cancel := context.WithTimeout(ctx, 10*time.Millisecond)
		defer cancel()
		cancelled := make(chan struct{})
		err := api.Within(short, 1000, func(c context.Context) {
			<-c.Done()
			close(cancelled)
		})
		check(errors.Is(err, context.DeadlineExceeded), "a call waiting on a Go function ends at its deadline")
		select {
		case <-cancelled:
		case <-time.After(5 * time.Second):
			check(false, "dropping a future Rust polls cancels its context")
		}
	}
	{
		// The body's context carries the caller's values and deadline.
		type key struct{}
		caller, cancel := context.WithTimeout(context.WithValue(ctx, key{}, "caller"), time.Hour)
		want, _ := caller.Deadline()
		var value any
		var deadline time.Time
		ok(api.Within(caller, 1000, func(c context.Context) {
			value = c.Value(key{})
			deadline, _ = c.Deadline()
		}))
		cancel()
		check(value == "caller" && deadline.Equal(want), "a Go function runs under the caller's context")
	}
	{
		// A body that returns once the caller cancelled, even one that ignores its
		// context, ends unfinished: the call reports the cancel, never Rust's success.
		caller, cancel := context.WithCancel(ctx)
		_, err := api.After(caller, func(context.Context) { cancel() }, 1)
		check(errors.Is(err, context.Canceled), "a caller's cancel the body sees reports the cancel")
	}
	{
		// The caller's cancel reaches the body's context.
		caller, cancel := context.WithCancel(ctx)
		started, cancelled := make(chan struct{}), make(chan struct{})
		got := make(chan error)
		go func() {
			_, err := api.After(caller, func(c context.Context) {
				close(started)
				<-c.Done()
				close(cancelled)
			}, 1)
			got <- err
		}()
		// Cancelled before its first poll, the call would never start the body.
		<-started
		cancel()
		check(errors.Is(<-got, context.Canceled), "a cancelled call waiting on a Go function reports the cancel")
		<-cancelled
	}

	// A waiter's poll of a Go function, made of each poll's waker, which the future owns.
	{
		ran := make(chan struct{}, 1)
		ok(api.WaiterPollFuture(ctx, func(context.Context) { ran <- struct{}{} }))
		check(len(ran) == 1, "a waiter polls a Go function to its end")
	}
	{
		gate := make(chan struct{})
		got := make(chan error)
		go func() { got <- api.WaiterPollFuture(ctx, func(context.Context) { <-gate }) }()
		select {
		case <-got:
			check(false, "a waiter's poll waits on a Go function that blocks")
		case <-time.After(10 * time.Millisecond):
		}
		close(gate)
		check(<-got == nil, "a Go function's return wakes the waiter's poll")
	}
	{
		// Dropped before it finishes, it cancels the Go function it owns.
		short, cancel := context.WithTimeout(ctx, 10*time.Millisecond)
		defer cancel()
		cancelled := make(chan struct{})
		err := api.WaiterPollFuture(short, func(c context.Context) {
			<-c.Done()
			close(cancelled)
		})
		check(errors.Is(err, context.DeadlineExceeded), "a waiter's poll ends at its deadline")
		select {
		case <-cancelled:
		case <-time.After(5 * time.Second):
			check(false, "dropping a waiter's poll cancels the Go function it owns")
		}
	}
	fmt.Println("futures ok")
}

// owned checks what frees an object: Close at once, or else the GC once it collects the
// object.
func owned() {
	for n := 2047; n <= 2049; n++ {
		payload := make([]byte, n)
		for i := range payload {
			payload[i] = byte(i)
		}
		packet := api.NextPacket(api.Packet{Seq: 1, Payload: rt.NewBytes(payload)})
		check(packet.Seq == 2 && string(packet.Payload.Copy()) == string(payload), fmt.Sprintf("a record's %d bytes cross both ways", n))
	}
	before := api.Dropped()
	token := api.NewToken(2)
	token.Close()
	check(api.Dropped() == before+1, "closing frees at once")
	token.Close()
	check(api.Dropped() == before+1, "closing again frees nothing")

	unclosed()
	collected(before+2, "the GC frees what nobody closed")
	runtime.GC()
	check(api.Dropped() == before+2, "the GC frees each once")

	before = api.Dropped()
	forward()
	collected(before+1, "a forwarded payload drops once nothing reaches it")
	uncollected(before+1, "a forwarded payload drops once")
	fmt.Println("owned ok")
}

// forward reads a payload Rust returned and writes it back, whole, sliced, and in a
// record: Rust takes a second owner of its own buffer each time, so the bytes stay where
// Rust put them, in a frame of its own so nothing keeps them reachable after.
//
//go:noinline
func forward() {
	ctx := context.Background()
	channel := api.NewChannel()
	defer channel.Close()
	// Long enough that Rust's buffer is lent rather than copied.
	payload := api.TokenBytes(4, 1024)
	at := addr(payload)
	channel.Send(payload)
	got := must(channel.Recv(ctx))
	check(addr(got) == at && got.Len() == 1024, "a payload sent back is Rust's own")
	channel.Send(got.Slice(1, 1023))
	got = must(channel.Recv(ctx))
	check(addr(got) == at+1 && got.Len() == 1022, "a slice of a payload sent back is Rust's own")
	packet := api.NextPacket(api.Packet{Seq: 1, Payload: got})
	check(addr(packet.Payload) == at+1 && packet.Payload.Len() == 1022, "a record carries a payload back as Rust's own")
}

// addr is where b's bytes start, as an io.Writer sees them.
func addr(b rt.Bytes) uintptr {
	var w addrWriter
	_, _ = b.WriteTo(&w)
	return uintptr(w)
}

type addrWriter uintptr

func (w *addrWriter) Write(p []byte) (int, error) {
	*w = addrWriter(unsafe.Pointer(&p[0]))
	return len(p), nil
}

// unclosed makes a token in a frame of its own, so nothing keeps it reachable.
//
//go:noinline
func unclosed() {
	api.NewToken(3)
}

// collected waits for the GC to have dropped Rust's values until dropped counts n.
func collected(n uint32, what string) {
	deadline := time.Now().Add(10 * time.Second)
	for api.Dropped() < n {
		check(time.Now().Before(deadline), what)
		runtime.GC()
		runtime.Gosched()
	}
}

// uncollected checks the GC drops nothing of Rust's while what it frees is reachable.
func uncollected(n uint32, what string) {
	for range 5 {
		runtime.GC()
		runtime.Gosched()
		time.Sleep(time.Millisecond)
	}
	check(api.Dropped() == n, what)
}

// closed checks that every call taking a closed object returns ErrClosed, however it was
// closed, and that a second Close does nothing.
func closed() {
	ctx := context.Background()
	panics := func(f func()) (panicked any) {
		defer func() { panicked = recover() }()
		f()
		return nil
	}
	isClosed := func(err error, name, what string) {
		check(errors.Is(err, rt.ErrClosed) && err.Error() == name+" is closed", what)
	}

	// A method, a trait's, and a field's accessors.
	c := api.NewCounter("c", 1)
	c.Close()
	c.Close()
	_, err := c.Get()
	isClosed(err, "Counter", "a method of a closed object")
	_, err = c.Clone()
	isClosed(err, "Counter", "Clone of a closed object")
	check(c.GoString() == "Counter is closed", "GoString of a closed object is the error")
	tally := api.NewTally()
	tally.Close()
	_, err = api.NewTally().Equal(tally)
	isClosed(err, "Tally", "Equal with a closed object")
	registry := must(api.NewRegistry(api.NewCounter("first", 1)))
	registry.Close()
	_, err = registry.FirstGet()
	isClosed(err, "Registry", "a field's getter of a closed object")
	// A call that fails moves nothing in.
	second := api.NewCounter("second", 2)
	isClosed(registry.FirstSet(second), "Registry", "a field's setter of a closed object")
	check(must(second.Get()) == 2, "a failed call leaves what it would move")

	// A record's method, whose receiver holds a value only Rust makes: Go's zero value of
	// one panics before Rust reads it.
	stamped := api.Stamped{At: api.StampFromMillis(2500), Count: 3}
	check(stamped.Total() == 5, "a record's method reads the value it holds")
	check(panics(func() { api.Stamped{Count: 3}.Total() }) == "a zero Stamp, which only Rust makes", "a zero value panics")

	// An argument, alone, or nested where the call holds it through its arena.
	_, err = api.BumpBoth(api.NewTally(), tally)
	isClosed(err, "Tally", "a closed argument")
	open := api.NewCounter("open", 3)
	_, err = api.Worth(&[]*api.Counter{open, c}, nil)
	isClosed(err, "Counter", "a closed object in a list")
	check(must(open.Get()) == 3, "a failed call leaves the rest of a list")

	// An async call.
	channel := api.NewChannel()
	channel.Close()
	_, err = channel.Recv(ctx)
	isClosed(err, "Channel", "an async call of a closed object")

	// Closed by a call that moved it, plain or boxed, or once the callback it was lent to
	// returned.
	moved := api.NewCounter("m", 0)
	must(moved.IntoLabel(nil))
	_, err = moved.Get()
	isClosed(err, "Counter", "a method of a moved object")
	tail := must(api.NewNode(1, nil))
	must(api.NewNode(2, tail))
	_, err = tail.Sum()
	isClosed(err, "Node", "a method of a boxed moved object")
	var kept *api.Counter
	must(must(api.NewRegistry(api.NewCounter("c", 0))).Edit(func(counter *api.Counter) bool {
		kept = counter
		return false
	}))
	_, err = kept.Get()
	isClosed(err, "Counter", "a method of a lent object kept past its callback")

	// Close racing calls on other goroutines: each call either runs, holding the handle
	// so Rust's value outlives it, or returns ErrClosed; Rust's is dropped once.
	before := api.Dropped()
	token := api.NewToken(5)
	var wg sync.WaitGroup
	var ran, failed atomic.Int64
	for range 8 {
		wg.Add(1)
		go func() {
			defer wg.Done()
			for {
				id, err := token.IdGet()
				if err != nil {
					isClosed(err, "Token", "a call racing Close")
					failed.Add(1)
					return
				}
				check(id == 5, "a call racing Close reads the live value")
				ran.Add(1)
			}
		}()
	}
	for ran.Load() < 100 {
		runtime.Gosched()
	}
	token.Close()
	wg.Wait()
	check(failed.Load() == 8 && api.Dropped() == before+1, "Close racing calls drops Rust's value once")
	fmt.Println("closed ok")
}

// bytes checks that Bytes Rust returns read Rust's buffer in place, kept alive by any view
// or reader of it and freed by the GC once none is reachable.
func bytes() {
	budgeted()
	// Under 512 B, copied and Rust's buffer freed at the call; from 512 B, lent.
	before := api.Dropped()
	small := api.TokenBytes(1, 511)
	check(api.Dropped() == before+1 && small.Len() == 511 && small.At(510) == 1, "bytes under 512 B are copied and Rust's freed")
	before++
	b := api.TokenBytes(1, 512)
	check(api.Dropped() == before, "returned bytes from 512 B keep Rust's buffer")
	want := strings.Repeat("\x01", 512)
	check(b.Len() == 512 && b.At(511) == 1 && string(b.Copy()) == want, "returned bytes read back")
	dst := make([]byte, 600)
	check(b.CopyTo(dst) == 512 && string(dst[:512]) == want, "CopyTo copies them all")
	sub := b.Slice(487, 512)
	check(sub.Len() == 25 && string(sub.Slice(5, 10).Copy()) == "\x01\x01\x01\x01\x01", "a sub-view reads its range")
	check(b.Slice(3, 3).Len() == 0, "an empty sub-view is empty")
	var out strings.Builder
	reader := b
	n, err := reader.WriteTo(&out)
	check(err == nil && n == 512 && out.String() == want && reader.Len() == 0, "WriteTo writes and consumes them")
	reader = b
	read, err := io.ReadAll(&reader)
	check(err == nil && string(read) == want && b.Len() == 512, "Read reads and consumes a copy of the view only")
	runtime.KeepAlive(b)
	collected(before+1, "the GC frees lent bytes once nothing reaches them")

	// Kept alive by a sub-view alone, then freed once nothing reaches it. 64 KiB are lent
	// in a batch of their own, so nothing else lent keeps them.
	view := subView(2)
	uncollected(before+1, "a sub-view keeps Rust's buffer")
	check(view.Len() == 10 && view.At(9) == 2, "a sub-view reads Rust's buffer after its parent is gone")
	view = rt.Bytes{}
	collected(before+2, "the GC frees bytes no view reaches")

	// Kept alive by a reader alone.
	r := bufferedReader(3)
	uncollected(before+2, "a reader keeps Rust's buffer")
	all, err := io.ReadAll(r)
	check(err == nil && len(all) == 64<<10 && all[64<<10-1] == 3, "a reader reads Rust's buffer after the Bytes are gone")
	r = nil
	collected(before+3, "the GC frees bytes no reader reaches")

	// Lent in batches, each freed once none of its Bytes is reachable.
	kept := batched()
	for range 5 {
		runtime.GC()
		runtime.Gosched()
	}
	check(api.Dropped() <= before+3+36, "the GC frees no batch whose Bytes are reachable")
	for i, k := range kept {
		check(k.Len() == 512 && k.At(511) == uint8(10*i), "kept bytes read Rust's buffer after their batch's others are gone")
	}
	kept = nil
	collected(before+43, "the GC frees every batch once none of its Bytes is reachable")

	// Bytes an object returns outlive its Close: no Bytes depends on an object.
	body := strings.Repeat("b", 2048)
	stamp := api.StampFromMillis(1)
	message := api.NewMessage(&stamp, rt.NewBytes([]byte(body)))
	payload := must(message.PayloadGet())
	message.Close()
	runtime.GC()
	check(string(payload.Copy()) == body, "bytes an object returned outlive its Close")
	fmt.Println("bytes ok")
}

// budgeted checks that at most 64 MiB of Rust's bytes are lent at once, freed or not, since
// the GC sees none of them: past that, bytes are copied, Rust's freed at the call, until the
// GC frees what was lent. First, so no bytes an earlier step lent count against it.
func budgeted() {
	before := api.Dropped()
	lentToBudget(before)
	collected(before+65, "the GC frees bytes lent up to the budget")
	check(api.TokenBytes(3, 2048).At(2047) == 3 && api.Dropped() == before+65, "bytes are lent again once the GC frees them")
	collected(before+66, "the GC frees bytes lent once the budget is free")
}

// lentToBudget lends 64 MiB in a frame of its own, so nothing keeps them reachable after.
//
//go:noinline
func lentToBudget(before uint32) {
	const mib = 1 << 20
	held := make([]rt.Bytes, 64)
	for i := range held {
		held[i] = api.TokenBytes(1, mib)
	}
	check(api.Dropped() == before, "64 MiB of Rust's bytes are lent")
	copied := api.TokenBytes(2, mib)
	check(api.Dropped() == before+1, "bytes past 64 MiB lent are copied, Rust's freed at the call")
	check(copied.Len() == mib && copied.At(0) == 2 && copied.At(mib-1) == 2 && held[63].At(mib-1) == 1, "bytes past the budget read as lent ones do")
}

// subView is a sub-view of returned bytes, whose Bytes nothing else keeps.
//
//go:noinline
func subView(id uint32) rt.Bytes {
	return api.TokenBytes(id, 64<<10).Slice(64<<10-10, 64<<10)
}

// bufferedReader is a reader of returned bytes, whose Bytes nothing else keeps.
//
//go:noinline
func bufferedReader(id uint32) io.Reader {
	b := api.TokenBytes(id, 64<<10)
	return bufio.NewReader(&b)
}

// batched lends 40 Bytes, more than a batch holds, keeping only every tenth.
//
//go:noinline
func batched() []rt.Bytes {
	var kept []rt.Bytes
	for i := range 40 {
		b := api.TokenBytes(uint32(i), 512)
		if i%10 == 0 {
			kept = append(kept, b)
		}
	}
	return kept
}

// broken breaks a contract, which must fail with a message naming it.
func broken(which string) {
	switch which {
	case "outlived":
		meters := api.NewMeters()
		first := must(meters.First())
		ok(meters.Clear())
		_ = must(first.Count())
	case "lent moved":
		must(api.NewRegistry(api.NewCounter("c", 0))).Edit(func(counter *api.Counter) bool {
			counter.IntoLabel(nil)
			return false
		})
	// An IPv6 address where Rust takes `SocketAddrV4`.
	case "family":
		any := netip.MustParseAddrPort("[::]:1")
		api.Nexts(any, any)
	// No Rust string holds text that is not UTF-8.
	case "text":
		api.Greet("\xff", api.ShapeSquare)
	case "callback panicked":
		api.Emit(panicking{}, rt.NewBytes([]byte{1}))
	// A Rust Duration longer than Go's longest.
	case "duration":
		api.Extended(math.MaxInt64, 1)
	// The packages serve wasm32 too, whose items the host's library lacks.
	case "target":
		api.WasmOnly()
	case "variant":
		api.PointerBits(api.PlatformWasm{Bits: 32})
	case "plain variant":
		api.IsFamily(api.FamilyWasm)
	case "nil reader":
		api.Tail(nil, 1)
	// An instant Go closed, which Rust cannot copy out.
	case "returned zero":
		api.Drive(context.Background(), api.Now(), func(std.Instant, *api.Waiter) (*std.Instant, error) {
			return &std.Instant{}, nil
		})
	default:
		check(false, "no case "+which)
	}
}

// cancelLog cancels its context at each line, and counts them.
type cancelLog struct {
	cancel context.CancelFunc
	lines  atomic.Int32
}

func (l *cancelLog) Log(uint8, string) bool {
	l.cancel()
	l.lines.Add(1)
	return true
}

type panicking struct{}

func (panicking) Log(uint8, string) bool {
	panic("a log failed")
}

func main() {
	switch {
	case len(os.Args) < 2:
		script()
	case os.Args[1] == "futures":
		futures()
	case os.Args[1] == "owned":
		owned()
	case os.Args[1] == "closed":
		closed()
	case os.Args[1] == "bytes":
		bytes()
	case os.Args[1] == "closures":
		closures()
	default:
		broken(os.Args[1])
	}
}
