// moq.c's script through the generated Go: a broadcast's track, published and subscribed
// to in one process, with every read a blocking call taking a context, and an origin, with
// moq-pattern, moq-clock, moq-tokio, hang, and moq-mux exported into the same library, and
// the catalog lent to Go for a call.
//
// The local reads need no runtime: each read that must wait blocks its goroutine until the
// write that wakes it. The origin's driver runs on a goroutine of its own, as long as the
// origin lives. A session over QUIC runs on moq-tokio's fallback runtime, whose threads wake
// Go's.
package main

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"net/netip"
	"os"
	"slices"
	"strings"
	"sync"
	"time"

	"moqtest/moq"
)

var failures = 0

func expect(ok bool, what string) {
	if !ok {
		fmt.Fprintf(os.Stderr, "failed: %s\n", what)
		failures++
	}
}

// must is v, failing the script on err.
func must[T any](v T, err error) T {
	if err != nil {
		panic(err)
	}
	return v
}

// pending is a context that reports a call waiting on it: a call takes Done only once a
// poll pends, so waiting closes once the call waits for a wake.
type pending struct {
	context.Context
	waiting chan struct{}
	once    sync.Once
}

func newPending() *pending {
	return &pending{Context: context.Background(), waiting: make(chan struct{})}
}

func (p *pending) Done() <-chan struct{} {
	p.once.Do(func() { close(p.waiting) })
	return p.Context.Done()
}

// result is what a call on a goroutine of its own returns, once it has.
func result[T any](call func() T) <-chan T {
	out := make(chan T, 1)
	go func() { out <- call() }()
	return out
}

// within is what out yields, or fails the script after a while.
func within[T any](out <-chan T, what string) T {
	select {
	case v := <-out:
		return v
	case <-time.After(10 * time.Second):
		panic("stalled: " + what)
	}
}

// drive runs an origin's driver on a goroutine that lives as long as the origin: it polls
// at the time with a waiter made from a waker, held across polls, since what a poll
// registers lives only as long as its waiter, and again on each wake and at the deadline
// the poll returns. Its result is the error the driver finished with.
func drive(driver *moq.OriginDriver) <-chan error {
	finished := make(chan error, 1)
	woken := make(chan struct{}, 1)
	waiter := moq.NewWaiter(func() {
		select {
		case woken <- struct{}{}:
		default:
		}
	})
	go func() {
		defer driver.Close()
		defer waiter.Close()
		for {
			next, err := driver.Poll(moq.Now(), waiter)
			if err != nil {
				finished <- err
				return
			}
			var deadline <-chan time.Time
			if next != nil {
				deadline = time.After(next.SaturatingDurationSince(moq.Now()))
				next.Close()
			}
			select {
			case <-woken:
			case <-deadline:
			}
		}
	}()
	return finished
}

func script() {
	ctx := context.Background()

	broadcast := moq.NewBroadcastInfo().Produce()
	track := must(broadcast.CreateTrack("video", nil))
	expect(track.Name() == "video", "a track has its name")

	// A second track of the same name is an error, printed as Rust prints it.
	_, err := broadcast.CreateTrack("video", nil)
	var merr *moq.Error
	expect(errors.As(err, &merr) && err.Error() == "duplicate", "a duplicate track's error is Rust's")

	subscriber := must(track.Consume().Subscribe(ctx, nil))

	// A group's arrival wakes the subscriber's pending receive.
	waitGroup := newPending()
	group := result(func() *moq.GroupConsumer { return must(subscriber.RecvGroup(waitGroup)) })
	<-waitGroup.waiting
	writer := must(track.AppendGroup())
	first := within(group, "a group's arrival wakes the receive")
	expect(first.Index() == 0, "the first group is 0")

	// So does a frame's.
	waitFrame := newPending()
	frame := result(func() *moq.FrameFrame { return must(first.ReadFrame(waitFrame)) })
	<-waitFrame.waiting
	expect(writer.WriteFrame(nil, []byte("hello")) == nil, "a frame writes")
	hello := within(frame, "a frame's arrival wakes the read")
	expect(string(hello.Payload()) == "hello", "a frame has its payload")
	expect(hello.Timestamp() == nil, "an untimed frame has no timestamp")
	// When it arrived: moq-clock's `Instant`, from moq-clock's own export.
	expect(moq.Now().CheckedDurationSince(hello.Arrived()) != nil, "a frame arrived before now")

	// A timestamp is a `Copy` value behind a handle: passing it copies it out. The track is
	// untimed, so a timestamped frame is refused.
	at := must(moq.TimestampFromMillis(1500))
	expect(at.AsSecs() == 1, "1500 ms is 1 s")
	err = writer.WriteFrame(at, []byte("late"))
	expect(errors.As(err, &merr) && err.Error() == "frame timestamp doesn't fit its track", "the late frame's error is Rust's")
	expect(at.AsSecs() == 1, "a copied timestamp stays usable")

	// The last frame, bytes that are not UTF-8, then the group's end, which reads as nil.
	world := append([]byte{0xff}, "world"...)
	expect(writer.WriteFrame(nil, world) == nil, "the last frame writes")
	expect(writer.Finish() == nil, "a group finishes")
	last := must(first.ReadFrame(ctx))
	expect(last != nil && bytes.Equal(last.Payload(), world), "the last frame reads")
	expect(must(first.ReadFrame(ctx)) == nil, "a finished group reads nil")

	// An origin and the driver that runs it, a pair. Its root, which it lends, is returned
	// as a new handle to a clone.
	origin, driver := moq.NewOriginProducer(moq.OriginConfigDefault())
	finished := drive(driver)
	expect(origin.Root().AsStr() == "", "an origin's root is empty")

	// Scoped beneath a root to patterns, which moq-pattern's own export builds.
	patterns := moq.NewPatterns()
	expect(patterns.Insert(must(moq.PatternSubtree("alice"))), "a pattern inserts")
	scoped := must(origin.Scope("room", patterns))
	expect(scoped.Root().AsStr() == "room", "a scope has its root")
	expect(must(scoped.Absolute("alice")).AsStr() == "room/alice", "a scope's paths are absolute")
	allowed := scoped.Allowed()
	expect(allowed.Matches("alice/cam"), "a scope allows its patterns")
	expect(!allowed.Matches("bob"), "a scope allows nothing else")
	consumer := scoped.Consume()
	expect(must(consumer.Absolute("bob")).AsStr() == "room/bob", "a scope's consumer shares its root")
	consumer.Close()
	scoped.Close()
	patterns.Close()

	// A broadcast the origin creates is stamped with its path, which its info lends. Each
	// handle to it is closed, since an open one keeps it, and so the origin, alive.
	publication := must(origin.CreateBroadcast("room/alice"))
	created := publication.Broadcast()
	path := created.Info().Path()
	created.Close()
	publication.Close()
	expect(path.AsStr() == "room/alice", "a broadcast has its path")

	// Paths own their segments, so Go builds them, and relative ones, from a string.
	_, err = moq.NewPath("/cam/")
	var invalid moq.PathInvalid
	expect(errors.As(err, &invalid) && invalid == moq.PathInvalidEmptySegment, "an empty segment is its own error")
	cam := must(moq.NewPath("cam"))
	expect(cam.AsStr() == "cam", "a path builds from its segments")
	joined := must(path.Join(cam))
	expect(joined.AsStr() == "room/alice/cam", "paths join")
	expect(joined.StripPrefix(path).Equal(cam), "a path strips its prefix")
	expect(path.StripPrefix(joined) == nil, "a path strips no longer prefix")
	expect(must(joined.Resolve(must(moq.NewRelative("../bob")))).AsStr() == "room/bob", "a relative path resolves")

	// A client publishes the origin through moq-net's generic `with_publisher`, at each
	// type that implements its bound, by reference, so Go keeps its handle.
	moq.NewClient().WithPublisherProducer(origin).Close()
	select {
	case err := <-finished:
		expect(false, "a driver runs while its origin lives: "+err.Error())
	default:
	}
	// The last owner's close wakes the driver, which then finishes, cleanly.
	origin.Close()
	err = within(finished, "the last owner's close wakes the driver")
	expect(errors.As(err, &merr) && err.Error() == "closed", "a finished driver's error is Rust's")

	// moq-mux reads an OpusHead into hang's catalog config: 44.1 kHz in, stereo, which the
	// catalog lists at the decoder's 48 kHz, keeping the head as its description.
	head := append([]byte("OpusHead"), 1, 2, 0x38, 0x01, 0x44, 0xac, 0, 0, 0, 0, 0)
	audio := must(moq.CodecOpusAudioConfig(head))
	expect(audio.SampleRate() == 48000, "Opus decodes at 48 kHz")
	expect(audio.ChannelCount() == 2, "the head is stereo")
	expect(audio.Codec() == moq.CatalogAudioCodecOpus{}, "the codec is Opus")
	description := audio.Description()
	expect(description != nil && bytes.Equal(*description, head), "the head is the description")
	// A truncated head is moq-mux's error.
	_, err = moq.CodecOpusAudioConfig(head[:8])
	var mux *moq.MuxError
	expect(errors.As(err, &mux) && err.Error() != "", "a truncated head's error has a message")

	// A subscription's ranges, from any iterable of them: a slice.
	groups := moq.TrackRangeGroups(moq.BoundsU64{
		Start: moq.BoundU64{Kind: moq.BoundIncluded, Value: 2},
		End:   moq.BoundU64{Kind: moq.BoundExcluded, Value: 6},
	})
	ranges := []moq.TrackRange{moq.TrackRangeLive(), groups}
	ranged := moq.TrackSubscriptionDefault().WithRanges(ranges).Ranges()
	expect(slices.EqualFunc(ranged, ranges, moq.TrackRange.Equal), "a subscription keeps its ranges")
	expect(groups.Start == moq.TrackAnchorAbsolute{V0: 2}, "groups start at their first")
	expect(groups.End != nil && *groups.End == moq.TrackAnchorAbsolute{V0: 6}, "groups end before their end")

	// A catalog at moq-mux's untyped extension, from the config Go builds, whose generic
	// `is_claimed` is named by the last segment of its argument, and whose `mutate` lends
	// the catalog to Go.
	publisher := moq.NewBroadcastInfo().Produce()
	extra := moq.CatalogConfigDefault().WithCatalogExtra(moq.HangCatalogExtraDefault())
	catalog := must(moq.NewCatalogProducerExtra(publisher, extra))
	expect(!catalog.IsClaimedBinaryConfig("state"), "an unclaimed name is not claimed")
	read := 0
	expect(catalog.Mutate(func(lent *moq.HangCatalogExtra) {
		if strings.HasPrefix(must(lent.ToJson()), "{") {
			read++
		}
	}) == nil, "mutate succeeds")
	expect(read == 1, "mutate lends the catalog once")

	// A JSON track the catalog lists, published through moq-mux's generic `json_snapshot` at
	// a JSON document, and a section Go parses from text, set in the lent catalog and read
	// back as text.
	status := must(catalog.JsonSnapshotConfig(must(publisher.CreateTrack("status", nil)), moq.JsonConfigDefault(), moq.JsonOptionsDefault()))
	expect(status.Name() == "status", "a JSON snapshot has its track's name")
	expect(catalog.IsClaimedJsonConfig("status"), "the JSON snapshot claims its name")
	// A value Go parses is published as the track's latest, which a subscriber reads back
	// as JSON text.
	on := must(moq.JsonParse(`{"on": true}`))
	expect(status.Update(on) == nil, "a JSON snapshot updates")
	on.Close()
	latest := must(must(status.Consume().RecvGroup(ctx)).ReadFrame(ctx))
	expect(latest != nil && string(latest.Payload()) == `{"on":true}`, "a subscriber reads the latest value")
	expect(catalog.Mutate(func(lent *moq.HangCatalogExtra) {
		sections := lent.Ext()
		expect(sections.Set("app", must(moq.JsonParse(`{"v": [1, true]}`))) == nil, "a section sets")
		lent.SetExt(sections)
	}) == nil, "mutate sets a section")
	var app *string
	expect(catalog.Mutate(func(lent *moq.HangCatalogExtra) {
		expect(strings.Contains(must(lent.ToJson()), `"status"`), "the catalog lists the JSON track")
		if section := lent.Ext().Get("app"); section != nil {
			text := section.ToText()
			app = &text
		}
	}) == nil, "mutate reads a section")
	expect(app != nil && *app == `{"v":[1,true]}`, "the catalog reads its section back as text")
	// Malformed text fails to parse, and a reserved section is moq-mux's error.
	_, err = moq.JsonParse(`{"v":`)
	var parse moq.StringError
	expect(errors.As(err, &parse) && parse.Value == "EOF while parsing a value at line 1 column 5", "a parse error is serde_json's")
	err = moq.CatalogHangExtraDefault().Set("json", must(moq.JsonParse("1")))
	expect(errors.As(err, &mux) && err.Error() == "reserved catalog section: json", "a reserved section is moq-mux's error")

	// moq-tokio's terminal connect errors, which moq-net's `Error` shares no names with.
	expect(moq.ConnectErrorUnauthorized.IsAuth(), "unauthorized is an auth error")
	expect(moq.ConnectErrorUnauthorized.String() == "unauthorized", "a connect error prints as Rust's")

	// Session config's foreign types. A URL is parsed from text, failing with why.
	_, err = moq.UrlParse("relay.example")
	expect(errors.As(err, &parse) && parse.Value == "relative URL without a base", "a URL's parse error is Rust's")
	// A session's config, its URL and pinned fingerprints set as Rust assigns them.
	connect := moq.ConnectConfigDefault()
	tls := connect.Tls()
	tls.SetFingerprint([]string{"ab:cd"})
	connect.SetTls(tls)
	tls.Close()
	connect.SetUrl(must(moq.UrlParse("https://relay.example:4443/demo")))
	expect(slices.Equal(connect.Tls().Fingerprint(), []string{"ab:cd"}), "a TLS config keeps its fingerprints")
	url := connect.Url()
	connect.Close()
	versions := moq.VersionsDefault()
	tokens := moq.NewAuthTokens(nil)
	dial := tokens.Dial(url, versions)
	dialed := dial.Url()
	expect(dialed.Equal(url), "a dial keeps its URL")
	expect(dialed.String() == "https://relay.example:4443/demo", "a URL prints as Rust's")
	dialed.Close()
	dial.Close()
	tokens.Close()
	url.Close()
	// A `Vec<String>`, as a slice.
	expect(len(moq.ALPNS()) > 0, "a constant slice crosses as a slice")
	alpns := versions.Alpns()
	expect(len(alpns) > 0 && !slices.Contains(alpns, ""), "each version has an ALPN")
	versions.Close()
	// A socket address, crossing in a listener's bind.
	bind := moq.ListenBind(moq.ListenBindAddr{V0: netip.MustParseAddrPort("[::1]:4443")})
	expect(fmt.Sprint(bind) == "[::1]:4443", "an IPv6 bind prints as Rust's")
	v4 := netip.MustParseAddrPort("127.0.0.1:0")
	expect(moq.ListenBindAddr{V0: v4}.String() == "127.0.0.1:0", "an IPv4 bind prints as Rust's")
	listen := moq.ListenConfigDefault()
	listen.SetBind(&bind)
	// The preferred address of each family, which takes only that family.
	listen.SetPreferredV4(&v4)
	preferred := listen.PreferredV4()
	expect(preferred != nil && *preferred == v4 && listen.PreferredV6() == nil, "a listener keeps its preferred IPv4 address")
	kept := listen.Bind()
	expect(kept != nil && *kept == bind, "a listener's config keeps its bind")
	listen.Close()
	// TLS files, as OS paths, and fingerprints, as strings: none by default.
	defaults := moq.TlsConnectDefault()
	expect(len(defaults.Root()) == 0 && len(defaults.Fingerprint()) == 0, "a default TLS config lists none")
	defaults.Close()

	session()
}

// session runs a session as moq.c does: a server bound to port 0 reads back the port it got
// and serves with one call, on a goroutine of its own, and a client dials it with one call
// and reads a frame back, each call blocking until moq-tokio's fallback runtime wakes it
// from its threads. A stall fails with the context's deadline.
func session() {
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()

	served := moq.OriginSpawn()
	live := must(served.CreateBroadcast("live"))
	expect(live.Announce(moq.OriginRouteDefault()) == nil, "a broadcast announces")
	liveTrack := must(live.Broadcast().CreateTrack("video", nil))
	liveGroup := must(liveTrack.AppendGroup())
	expect(liveGroup.WriteFrame(nil, []byte("hello")) == nil, "a served frame writes")
	expect(liveGroup.Finish() == nil, "a served group finishes")

	serverConfig := moq.ServerConfigDefault()
	listen := serverConfig.Listen()
	var bind moq.ListenBind = moq.ListenBindAddr{V0: netip.MustParseAddrPort("[::]:0")}
	listen.SetBind(&bind)
	tls := listen.Tls()
	tls.SetGenerate([]string{"localhost"})
	listen.SetTls(tls)
	serverConfig.SetListen(listen)
	server := must(serverConfig.Init())
	local := must(server.LocalAddr())
	expect(local.Port() != 0, "a server bound to port 0 reads back the port it got")
	serving, stop := context.WithCancel(ctx)
	defer stop()
	go server.ServePublish(serving, served.Consume())

	connect := moq.ConnectConfigDefault()
	connect.SetUrl(must(moq.UrlParse(fmt.Sprintf("moqt://localhost:%d", local.Port()))))
	insecure := connect.Tls()
	yes := true
	insecure.SetInsecure(&yes)
	connect.SetTls(insecure)
	clientConfig := moq.ClientConfigDefault()
	clientConfig.SetConnect(connect)
	client := must(clientConfig.Init())
	received := moq.OriginSpawn()
	remote := received.Consume()
	connection := client.Clone().WithReconnect(false).Consume(received)
	expect(connection != nil, "a client with a URL dials it")
	established := must(connection.Established(ctx))

	subscriber := must(must(must(remote.RequestBroadcast(ctx, "live", nil)).Track("video")).Subscribe(ctx, nil))
	frame := must(must(subscriber.RecvGroup(ctx)).ReadFrame(ctx))
	expect(frame != nil && string(frame.Payload()) == "hello", "a session reads the frame the server serves")

	expect(established.Close_(ctx) == nil, "a connection closes")
	expect(client.Close_(ctx) == nil, "a client closes")
}

func main() {
	script()
	if failures != 0 {
		os.Exit(1)
	}
	fmt.Println("ok")
}
