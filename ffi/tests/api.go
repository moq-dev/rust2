// The api fixture's Go test: the script fixtures/api/examples/transcript.rs runs natively,
// through the generated Go. tests/fixtures.rs compares the transcripts.
//
// With an argument, it instead checks the futures' semantics, what closing and the GC
// free, or breaks a contract, which must fail.
package main

import (
	"context"
	"errors"
	"fmt"
	"math"
	"net/netip"
	"os"
	"runtime"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	"apitest/api"
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

func quoted(s string) string {
	return fmt.Sprintf("%q", s)
}

func number[T any](n T) string {
	return fmt.Sprint(n)
}

func bound(b api.BoundU64) string {
	switch b.Kind {
	case api.BoundIncluded:
		return fmt.Sprintf("Included(%d)", b.Value)
	case api.BoundExcluded:
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
	fmt.Printf("get %d\n", c.Get())
	fmt.Printf("name %s\n", c.Name())
	fmt.Printf("add ok %d\n", must(c.Add(10)))
	// An exported error type is returned as itself.
	var e api.Error
	_, err := c.Add(2000)
	check(errors.As(err, &e), "add returns an api.Error")
	fmt.Printf("add err %s %s\n", e.Error(), e.GoString())
	_, err = c.Sub(20)
	check(errors.As(err, &e), "sub returns an api.Error")
	fmt.Printf("sub err %s %s\n", err, e.GoString())
	c.Rename("d")
	fmt.Printf("name %s\n", c.Name())
	fmt.Printf("is d %t\n", c.Is("d"))
	cp := c.Clone()
	must(cp.Add(1))
	fmt.Printf("clone %d original %d\n", cp.Get(), c.Get())
	cp.Close()
	fmt.Printf("debug %s\n", c.GoString())
	label := c.IntoLabel(&p)
	fmt.Printf("label %s\n", label.GoString())
	fmt.Printf("note %t %t\n", api.HasText(api.Note{Text: ptr("")}), api.HasText(api.NoteDefault()))

	below, above := api.NewCounter("s", 7).Split(5)
	fmt.Printf("split %s %d %s %d\n", below.Name(), below.Get(), above.Name(), above.Get())
	one := api.Only(api.NewCounter("o", 3))
	fmt.Printf("only %s %d\n", one.Name(), one.Get())
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
		if found := api.Find([]byte("hello"), []byte(needle)); found != nil {
			fmt.Printf("find %d %d\n", found.V0, found.V1)
		} else {
			fmt.Println("find none")
		}
	}
	rows := api.Rows([]api.Point{{X: 1, Y: 2}, {X: 3, Y: 3}})
	shapes := map[api.Shape]string{api.ShapeCircle: "Circle", api.ShapeSquare: "Square"}
	fmt.Printf("rows %s\n", strings.Trim(list(rows, func(r api.Tuple3[string, api.Shape, api.Point]) string {
		return fmt.Sprintf("%s %s %d %d", r.V0, shapes[r.V1], r.V2.X, r.V2.Y)
	}), "[]"))
	// Each handle in a tuple moves in, as one taken by value does.
	board := api.NewBoard([]api.Tuple2[string, *api.Counter]{
		{V0: "low", V1: api.NewCounter("l", 1)},
		{V0: "high", V1: api.NewCounter("h", 9)},
	})
	fmt.Printf("ranked %s\n", strings.Trim(list(board.Ranked(), func(r api.Tuple2[string, *api.Counter]) string {
		return fmt.Sprintf("%s %s %d", r.V0, r.V1.Name(), r.V1.Get())
	}), "[]"))
	board.Close()
	square := api.ShapeSquare
	span := api.Span{Bounds: api.Tuple2[uint32, uint32]{V0: 2, V1: 10}, Named: &api.Tuple2[string, api.Shape]{V0: "s", V1: square}}
	lower, upper := api.Halves(span)
	fmt.Printf("halves %d %d %d %d %s\n", lower.V0, lower.V1, upper.V0, upper.V1, span.GoString())
	n, text := api.Swapped(api.Tuple2[string, uint32]{V0: "t", V1: 4})
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
	fmt.Printf("lent back %s boxed %s\n", list(api.LentBack([]byte{1, 2}), number), list(api.BoxedTuple([]byte{3}), number))
	// A borrowed string and an owned one share a tuple struct.
	check(api.Title(false) == nil, "an informal title is none")
	fmt.Printf("title Some((%s,)) None named Some((%s,))\n", quoted(*api.Title(true)), quoted(*api.Named("x")))

	// Each lent counter is a new handle to a clone.
	registry := api.NewRegistry(api.NewCounter("first", 1))
	fmt.Printf("last %t\n", registry.Last() != nil)
	registry.Push(api.NewCounter("last", 2))
	first := registry.First()
	must(first.Add(10))
	fmt.Printf("first %d lent %d\n", registry.First().Get(), first.Get())
	last := registry.Last()
	must(last.Add(10))
	fmt.Printf("last %d lent %d\n", registry.Last().Get(), last.Get())
	// A closure lent for the call edits the first counter through a handle.
	by := uint64(5)
	edited := registry.Edit(func(counter *api.Counter) bool {
		must(counter.Add(by))
		return counter.Edited() && counter.Get() == 6
	})
	fmt.Printf("edit %t %d\n", edited, registry.First().Get())
	// A setter moves an object in, as assignment does, closing its handle.
	registry.SetFirst(api.NewCounter("second", 3))
	fmt.Printf("set first %d %s\n", registry.First().Get(), registry.First().Name())
	fmt.Printf("walk %d\n", api.Walk(3, func(cursor *api.Cursor) uint64 { return cursor.At() * 2 }))
	fmt.Printf("each %d\n", api.Each(2, func(at uint32) { fmt.Printf("visit %d\n", at) }))
	fmt.Printf("offset %d\n", api.Offset(func() uint32 { return 7 }, 3))
	fmt.Printf("shift %d\n", api.Shift(func() uint32 { return 7 }, 4))

	name := api.NameOwned("a")
	joined := name.Join(api.NameOwned("b"))
	fmt.Printf("joined %s %s %t\n", joined.AsStr(), joined.GoString(), joined.Equal(name))
	joined.Rename("c")
	fmt.Printf("renamed %s\n", joined.AsStr())
	long := api.Longer(joined, api.NameOwned("dd"))
	fmt.Printf("longer %s\n", long.AsStr())
	fmt.Printf("unnamed [%s]\n", api.Unnamed().AsStr())

	fmt.Printf("total %d\n", api.Total(api.RangeU64{Start: 2, End: 5}))
	wide := api.Widen(api.RangeU64{Start: 2, End: 5}, 1)
	fmt.Printf("widen %d %d\n", wide.Start, wide.End)
	included := func(n uint64) api.BoundU64 { return api.BoundU64{Kind: api.BoundIncluded, Value: n} }
	excluded := func(n uint64) api.BoundU64 { return api.BoundU64{Kind: api.BoundExcluded, Value: n} }
	unbounded := api.BoundU64{Kind: api.BoundUnbounded}
	counts := list([]api.BoundsU64{
		{Start: included(2), End: excluded(5)},
		{Start: included(2), End: included(5)},
		{Start: included(3), End: unbounded},
		{Start: unbounded, End: unbounded},
		{Start: excluded(2), End: included(4)},
	}, func(b api.BoundsU64) string { return fmt.Sprint(api.Count(b)) })
	fmt.Printf("count %s\n", strings.ReplaceAll(strings.Trim(counts, "[]"), ",", ""))
	fmt.Printf("end %s %s\n", bound(api.End(3)), bound(api.End(0)))
	start, end := api.Ends(3)
	fmt.Printf("ends %s %s\n", bound(start), bound(end))
	for _, n := range []uint64{2, 5} {
		got, err := api.Below(n, 3)
		var bounds api.BoundsU64Error
		switch {
		case err == nil:
			fmt.Printf("below %d\n", got)
		case errors.As(err, &bounds):
			fmt.Printf("below err %s %s\n", bound(bounds.Value.Start), bound(bounds.Value.End))
		default:
			check(false, "below returns its bounds")
		}
	}
	window := api.Window{Range: api.RangeU64{Start: 2, End: 8}, End: api.End(5)}
	fmt.Printf("window %t %t %s\n", window.Contains(4), window.Contains(6), window.GoString())

	fmt.Println(api.Greet("x", api.ShapeSquare))
	fmt.Printf("len %d\n", api.Len([]byte("abc")))
	binary := []byte{0xff, 0, 0x61}
	fmt.Printf("hex %s\n", api.Hex(binary))
	fmt.Printf("timed hex %s\n", api.TimedHex(binary))
	fmt.Printf("sample hex %s\n", api.SampleHex(binary))
	fmt.Printf("queued hex %s\n", api.QueuedHex(binary))
	fmt.Printf("tick %s\n", api.TickName(api.Label{Name: "tick"}))
	fmt.Printf("rooted %s\n", api.RootedName(api.Label{Name: "rooted"}))
	fmt.Printf("stored %s\n", api.StoredText("stored"))
	fmt.Printf("layer %s\n", api.LayerName(api.Label{Name: "layer"}))
	for _, event := range []api.Event{api.EventMoved{V0: p}, api.EventNamed{Name: "n"}, api.EventDone{}} {
		fmt.Println(api.Describe(event))
	}

	// A `Copy` handle passed by value is copied, so it stays usable.
	stamp := api.StampFromMillis(2500)
	fmt.Printf("secs %d\n", stamp.Secs())
	fmt.Printf("later %t\n", api.StampFromMillis(3000).Later(stamp))
	fmt.Printf("secs %d\n", stamp.Secs())
	message := api.NewMessage(stamp, []byte("body"))
	fmt.Printf("payload %s\n", message.Payload())
	fmt.Printf("stamp %s\n", message.Stamp().GoString())
	fmt.Printf("sent %t\n", message.Sent())
	// Setters assign in place: the bytes are copied in, and a `Copy` handle copied.
	reset := api.StampFromMillis(4000)
	message.SetStamp(reset)
	message.SetPayload([]byte("edited"))
	fmt.Printf("payload %s\n", message.Payload())
	fmt.Printf("stamp %s\n", message.Stamp().GoString())
	fmt.Printf("secs %d\n", reset.Secs())
	message.SetStamp(nil)
	fmt.Printf("stamp %t\n", message.Stamp() != nil)

	// Rust frees the implementation once it drops it, releasing Go's.
	fmt.Printf("kept %d\n", api.Emit(printLog{}, []byte{1, 2, 3, 4}))
	logger := api.NewLogger(printLog{})
	fmt.Printf("write %t\n", logger.Write("hi"))
	logger.Close()
	fmt.Println("dropped")
	// Rust calls it on another thread, before the call returns.
	threshold := uint8(2)
	filtered := api.Filter([]byte{1, 2, 3, 4}, func(level uint8, line string) bool {
		fmt.Printf("filter %d %s\n", level, line)
		return level > threshold
	})
	fmt.Printf("filtered %d\n", filtered)

	channel := api.NewChannel()
	// The receive waits once its future pends, until the send wakes it.
	watching := watch(ctx)
	got := make(chan []byte)
	go func() { got <- must(channel.Recv(watching)) }()
	<-watching.waiting
	fmt.Println("recv pending true")
	channel.Send([]byte("hi"))
	payload := <-got
	wakes := 1
	fmt.Printf("woken %d\n", wakes)
	fmt.Printf("recv %s\n", payload)
	channel.Send(nil)
	if _, err := channel.RecvNonempty(ctx); err != nil {
		fmt.Printf("nonempty err %s\n", err)
	}
	// The send happens at the call: the future is cancelled before its first poll.
	cancelled, cancel := context.WithCancel(ctx)
	cancel()
	check(errors.Is(channel.SendLater(cancelled, []byte("eager")), context.Canceled), "a cancelled call returns its context's error")
	fmt.Printf("recv %s\n", must(channel.Recv(ctx)))
	channel.Send([]byte("sized"))
	sized, size, err := channel.RecvSized(ctx)
	check(err == nil, "recv sized")
	fmt.Printf("sized %s %d\n", sized, size)
	// The waker moves into Rust, which keeps it until the send wakes it.
	channel.Notify(func() { wakes++ })
	channel.Send([]byte("rung"))
	fmt.Printf("woken %d\n", wakes)

	list1 := api.NewNode(1, nil)
	list1.Push(2)
	fmt.Printf("node %d\n", list1.Sum())

	tally := api.NewTally()
	tally.Bump()
	fmt.Printf("tally %d %t\n", tally.Bump(), tally.Equal(tally))
	fmt.Printf("both %d\n", api.BumpBoth(tally, tally))
	fmt.Printf("peek %d\n", api.Peek(tally, func(lent *api.Tally) uint32 { return lent.Bump() }))

	area := api.Grow(api.NewArea(api.Size{Width: 2, Height: 3}), 1)
	areaSize := area.Size()
	fmt.Printf("area %d %d %d\n", areaSize.Width, areaSize.Height, area.Pixels())

	// Generic types, each at the instances it infers, named by their arguments.
	pair := api.PairU32{Left: 2, Right: 5}
	swapped := pair.Swap()
	fmt.Printf("pair %d %d sum %d %s %t\n", swapped.Left, swapped.Right, pair.Sum(), pair.GoString(), pair.Equal(swapped))
	fmt.Printf("even %t %t\n", pair.Even(), api.PairU32{Left: 7, Right: 7}.Even())
	// An instance renamed alone, beside another of `Pair` that keeps its name.
	tagged := api.Tag(api.Size{Width: 1, Height: 2}, 3)
	flipped := tagged.Swap()
	fmt.Printf("sized %d %d %s\n", flipped.Left, flipped.Right.Width, flipped.GoString())
	beside := api.PairU32{Left: 258, Right: 9}.BesideSize(tagged.Left)
	fmt.Printf("beside %d %s\n", beside.Left, beside.GoString())
	// At its default, named as the type itself.
	stack := api.NewStack()
	for i, n := range []string{"a", "b"} {
		stack.Push(api.NewCounter(n, uint64(i)+1))
	}
	top := stack.Pop().Get()
	fmt.Printf("stack %d Some(%d) %t %t\n", stack.Depth(), top, stack.Pop() != nil, stack.Pop() == nil)
	// Generic functions and methods, at each implementation of the trait bounding them, by
	// reference, which leaves Go its handle.
	fromPoint := api.CounterSeededPoint("p", api.Point{X: -3, Y: 4})
	fromSize := api.CounterFromSize("s", api.Size{Width: 5, Height: 6})
	fromCounter := api.CounterSeededCounter("c", fromPoint)
	fromPolar := api.CounterSeededPolarPoint("q", api.PolarPoint{Turns: 1, Distance: 2})
	fmt.Printf("seeded %d %d %d %d past %d seeds %d\n", fromPoint.Get(), fromSize.Get(), fromCounter.Get(), fromPolar.Get(),
		stack.DepthPastPoint(api.Point{X: 1, Y: 2}), api.SeedsPointSize(api.Size{Width: 2, Height: 3}))

	// A trait implemented for `Point` and `&Point` apart: one instance.
	fmt.Printf("weigh %d\n", api.WeighPoint(api.Point{X: -1, Y: 2}))
	// Instances that name each other, and one made only by an impl of it.
	crossed := api.Cross(api.LeftRightU8{Value: api.RightU8{Value: 1}}, api.RightLeftU8{Value: api.LeftU8{Value: 2}})
	fmt.Printf("cross %d outer %d\n", crossed, api.NewOuterInnerU8(4).Get())
	// A bound that outlives the call takes a value, and an instance only a constant names.
	fmt.Printf("kept %d boxed %d\n", api.KeptPoint(api.Point{X: 1, Y: -2}), api.BOXED().Value)
	fmt.Printf("keep %d\n", api.KeepPoint(api.Point{X: 1, Y: -2}))
	// Inferred instances taking a lent closure and a foreign implementation, and one only
	// its impl of a trait names.
	sides := api.PairU32{Left: 2, Right: 5}
	fmt.Printf("apply %d\n", api.ApplyU32(sides, func(side uint32) uint32 { return side * 3 }))
	fmt.Printf("report %d\n", api.ReportU32(sides, printLog{}))
	fmt.Printf("wrapped %d\n", api.KeptWrapU32(api.WrapU32{Value: 4}))
	fmt.Printf("keeps %d\n", api.Keeps(printLog{}).Value)

	// Its Drop runs when Go closes the handle, not before.
	token := api.NewToken(7)
	fmt.Printf("token %d dropped %d\n", token.Id(), api.Dropped())
	token.Close()
	fmt.Printf("dropped %d\n", api.Dropped())

	// A field and the method of its name, each its own member.
	retry := api.NewRetry(ptr[uint32](5))
	limit := retry.LimitSetting()
	isSome := 0
	if limit != nil {
		isSome = 1
	}
	fmt.Printf("retry %d %d %d %d\n", isSome, *limit, retry.Limit(), retry.Delay())
	// The renamed field's setter, named as its getter.
	retry.SetLimitSetting(nil)
	fmt.Printf("retry %d\n", retry.Limit())
	fmt.Printf("slot %d\n", api.NewSlotU8(200).Value())

	// Foreign types: a URL parsed from text, socket addresses, OS paths, and slices.
	base := must(api.UrlParse("https://example.com/a/b"))
	fmt.Printf("url %s host Some(%s)\n", base, quoted(*api.Host(base)))
	url := must(api.Resolve(base.Clone(), "c?d"))
	fmt.Printf("resolve %s %t\n", url, url.Equal(base))
	url.Close()
	var parse api.StringError
	_, err = api.UrlParse("no scheme")
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
	fmt.Printf("chunks %s\n", list(api.Chunks([]byte("abcde"), 2), func(b []byte) string { return list(b, number) }))
	counters := api.Counters([]string{"x", "y"})
	fmt.Printf("counters %s\n", strings.Trim(list(counters, func(c *api.Counter) string {
		return fmt.Sprintf("%s %d", c.Name(), c.Get())
	}), "[]"))
	// Each counter moves in, so Go closes it.
	fmt.Printf("worth %d %d\n", api.Worth(&[]*api.Counter{counters[0]}, []*api.Counter{counters[1]}), api.Worth(nil, nil))
	fmt.Printf("sums %s\n", list(api.Sums([][]uint32{{1, 2}, {}, {3}}), number))
	sum := api.Summed([]api.Point{{X: 1, Y: 2}, {X: -4, Y: 6}})
	none := api.Summed(nil)
	fmt.Printf("summed %d %d %d %d\n", sum.X, sum.Y, none.X, none.Y)
	fmt.Printf("longest %d %d\n", api.Longest([]string{"a", "abc"}), api.Longest(nil))
	doc := must(api.JsonParse(`{"b": [1, 2.5, null], "a": "é"}`))
	fmt.Printf("json %s\n", api.Encode(doc))
	fmt.Printf("timed text %s\n", api.TimedText(doc))
	tagged2 := api.Tagged(doc, "t")
	fmt.Printf("tagged %s\n", tagged2.ToText())
	tagged2.Close()
	_, err = api.JsonParse(`{"a":`)
	check(errors.As(err, &parse), "parse returns its message")
	fmt.Printf("json parse err %s\n", err)
	site := api.NewSite(base)
	fmt.Printf("site %s %s\n", site.Home(), site.Home())

	// Slices, passed in and returned as clones, and the socket address of each family.
	fmt.Printf("shapes %s greetings %s\n", list(api.ShapeALL(), func(s api.Shape) string { return shapes[s] }), list(api.GREETINGS(), quoted))
	trail := api.NewTrail([]api.Point{{X: 1, Y: 2}, {X: 3, Y: 4}}, []string{"a", "b"})
	point := func(p api.Point) string { return p.GoString() }
	labeled := func(l api.Label) string { return l.GoString() }
	fmt.Printf("trail %s %s\n", list(trail.Points(), point), list(trail.Labels(), labeled))
	// Returned iterators, each collected at the call: drained labels are gone after.
	fmt.Printf("iterated %s %s %s\n", list(trail.Iter(), point), list(trail.Names(), quoted), list(trail.Bytes(), number))
	fmt.Printf("drained %s then %s\n", list(trail.Drain(), labeled), list(trail.Labels(), labeled))
	trail.Close()
	fmt.Printf("numbered %s\n", strings.Trim(list(api.Numbered([]string{"p", "q"}), func(c *api.Counter) string {
		return fmt.Sprintf("%s %d", c.Name(), c.Get())
	}), "[]"))
	next4, next6 := api.Nexts(v4, v6)
	fmt.Printf("nexts %s %d %d %s\n", next4.Addr(), next4.Port(), next6.Port(), next6.Addr().Zone())
}

// futures checks the futures' semantics beyond the transcript, each by an assertion.
func futures() {
	ctx := context.Background()
	channel := api.NewChannel()

	// The send lands from another goroutine's thread while the receive waits.
	go channel.Send([]byte("blocked"))
	check(string(must(channel.Recv(ctx))) == "blocked", "recv returns the payload another goroutine sent")

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
		channel.Send([]byte("kept"))
		cancel()
		close(gate)
		check(errors.Is(<-done, context.Canceled), "a receive cancelled after its wake returns its context's error")
		check(string(must(channel.Recv(ctx))) == "kept", "a cancelled receive loses nothing")
	}

	// A context done before the call takes nothing, even a payload ready at the first poll.
	{
		channel.Send([]byte("ready"))
		cancelled, cancel := context.WithCancel(ctx)
		cancel()
		_, err := channel.Recv(cancelled)
		check(errors.Is(err, context.Canceled), "a call with a done context returns its error")
		check(string(must(channel.Recv(ctx))) == "ready", "a call with a done context takes nothing")
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
		go func() { got <- string(must(other.Recv(watching))) }()
		<-watching.waiting
		sender := other.Clone()
		other.Close()
		sender.Send([]byte("held"))
		check(<-got == "held", "a closed handle stays alive in the call waiting on it")
		sender.Close()
	}
	fmt.Println("futures ok")
}

// owned checks what frees an object or bytes: Close at once, or else the GC once it
// collects the object. Bytes Rust returns are copied out, so Rust's are freed at the call.
func owned() {
	for n := 1023; n <= 1025; n++ {
		payload := make([]byte, n)
		for i := range payload {
			payload[i] = byte(i)
		}
		packet := api.NextPacket(api.Packet{Seq: 1, Payload: payload})
		check(packet.Seq == 2 && string(packet.Payload) == string(payload), fmt.Sprintf("a record's %d bytes cross both ways", n))
	}
	before := api.Dropped()
	bytes := api.TokenBytes(1, 1025)
	check(api.Dropped() == before+1, "returned bytes free Rust's at the call")
	check(string(bytes) == strings.Repeat("\x01", 1025), "returned bytes read back")

	token := api.NewToken(2)
	token.Close()
	check(api.Dropped() == before+2, "closing frees at once")
	token.Close()
	check(api.Dropped() == before+2, "closing again frees nothing")

	unclosed()
	deadline := time.Now().Add(10 * time.Second)
	for api.Dropped() < before+3 {
		check(time.Now().Before(deadline), "the GC frees what nobody closed")
		runtime.GC()
		runtime.Gosched()
	}
	runtime.GC()
	check(api.Dropped() == before+3, "the GC frees each once")
	fmt.Println("owned ok")
}

// unclosed makes a token in a frame of its own, so nothing keeps it reachable.
//
//go:noinline
func unclosed() {
	api.NewToken(3)
}

// broken breaks a contract, which must fail with a message naming it.
func broken(which string) {
	switch which {
	case "moved":
		c := api.NewCounter("c", 0)
		c.IntoLabel(nil)
		c.Get()
	case "boxed moved":
		tail := api.NewNode(1, nil)
		api.NewNode(2, tail)
		tail.Sum()
	case "lent moved":
		api.NewRegistry(api.NewCounter("c", 0)).Edit(func(counter *api.Counter) bool {
			counter.IntoLabel(nil)
			return false
		})
	case "lent kept":
		var kept *api.Counter
		api.NewRegistry(api.NewCounter("c", 0)).Edit(func(counter *api.Counter) bool {
			kept = counter
			return false
		})
		kept.Get()
	// An IPv6 address where Rust takes `SocketAddrV4`.
	case "family":
		any := netip.MustParseAddrPort("[::]:1")
		api.Nexts(any, any)
	// No Rust string holds text that is not UTF-8.
	case "text":
		api.Greet("\xff", api.ShapeSquare)
	case "callback panicked":
		api.Emit(panicking{}, []byte{1})
	// A Rust Duration longer than Go's longest.
	case "duration":
		api.Extended(math.MaxInt64, 1)
	default:
		check(false, "no case "+which)
	}
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
	default:
		broken(os.Args[1])
	}
}
