# moq.c's script through the generated Python: a broadcast's track, published and
# subscribed to in one process, with every read awaited, and an origin, its driver run on
# asyncio's loop, with moq-pattern, moq-clock, moq-tokio, hang, and moq-mux exported into
# the same library, and the catalog lent to Python for a call.
#
# A local read that must wait resumes once the write that wakes it lands, on the loop the
# script steps. A session over QUIC runs on moq-tokio's fallback runtime, whose threads
# wake the loop's.

import asyncio
import sys

import moq

failures = 0


def expect(ok, what):
	global failures
	if not ok:
		print(f"failed: {what}", file=sys.stderr)
		failures += 1


def text(view):
	"""The bytes a memoryview lends as text, releasing it."""
	with view:
		return bytes(view).decode()


def step(loop):
	"""Run the loop until what is ready has run."""
	loop.run_until_complete(asyncio.sleep(0))


async def drive(driver):
	"""Poll `driver` as asyncio's loop would: on each wake, and at the deadline each poll
	returns, with one waiter held across polls, since what a poll registers lives only as
	long as its waiter. It returns once the driver finishes, raising its error."""
	woken = asyncio.Event()
	waiter = moq.Waiter.new(moq.Waker(woken.set))
	while True:
		woken.clear()
		deadline = driver.poll(moq.now(), waiter)
		timeout = None if deadline is None else deadline.saturating_duration_since(moq.now()).total_seconds()
		try:
			await asyncio.wait_for(woken.wait(), timeout)
		except TimeoutError:
			pass


def local(loop):
	broadcast = moq.BroadcastInfo.new().produce()
	track = broadcast.create_track("video", None)
	expect(track.name() == "video", "a track has its name")

	# A second track of the same name is an error, printed as Rust prints it.
	try:
		broadcast.create_track("video", None)
		expect(False, "a duplicate track is an error")
	except moq.ErrorException as e:
		expect(str(e) == "duplicate", "a duplicate track's error is Rust's")

	subscriber = loop.run_until_complete(track.consume().subscribe(None))

	# A group's arrival wakes the subscriber's pending receive.
	recv = loop.create_task(subscriber.recv_group())
	step(loop)
	expect(not recv.done(), "a receive waits for a group")
	writer = track.append_group()
	expect(not recv.done(), "a receive never resumes inside the wake")
	first = loop.run_until_complete(asyncio.wait_for(recv, 5))
	expect(first.index() == 0, "the first group is 0")

	# So does a frame's.
	read = loop.create_task(first.read_frame())
	step(loop)
	expect(not read.done(), "a read waits for a frame")
	writer.write_frame(None, b"hello")
	hello = loop.run_until_complete(asyncio.wait_for(read, 5))
	expect(text(hello.payload) == "hello", "a frame has its payload")
	expect(hello.timestamp is None, "an untimed frame has no timestamp")
	# When it arrived: moq-clock's `Instant`, from moq-clock's own export.
	expect(moq.now().checked_duration_since(hello.arrived()) is not None, "a frame arrived before now")

	# A timestamp is a `Copy` value behind a handle: passing it copies it out. The track is
	# untimed, so a timestamped frame is refused.
	at = moq.Timestamp.from_millis(1500)
	expect(at.as_secs() == 1, "1500 ms is 1 s")
	try:
		writer.write_frame(at, b"late")
		expect(False, "a timestamped frame on an untimed track is an error")
	except moq.ErrorException as e:
		expect(str(e) == "frame timestamp doesn't fit its track", "the late frame's error is Rust's")
	expect(at.as_secs() == 1, "a copied timestamp stays usable")

	# The last frame, bytes that are not UTF-8, then the group's end, which reads as None.
	world = b"\xffworld"
	writer.write_frame(None, world)
	writer.finish()
	last = loop.run_until_complete(first.read_frame())
	expect(last is not None and bytes(last.payload) == world, "the last frame reads")
	expect(loop.run_until_complete(first.read_frame()) is None, "a finished group reads None")


def origin(loop):
	# An origin and the driver that runs it, a pair. Its root, which it lends, is returned
	# as a new object of a clone.
	origin, driver = moq.OriginProducer.new(moq.OriginConfig.default())
	expect(origin.root().as_str() == "", "an origin's root is empty")

	# Scoped beneath a root to patterns, which moq-pattern's own export builds.
	with moq.Patterns.new() as patterns:
		expect(patterns.insert(moq.Pattern.subtree("alice")), "a pattern inserts")
		with origin.scope("room", patterns) as scoped:
			expect(scoped.root().as_str() == "room", "a scope has its root")
			expect(scoped.absolute("alice").as_str() == "room/alice", "a scope's paths are absolute")
			allowed = scoped.allowed()
			expect(allowed.matches("alice/cam"), "a scope allows its patterns")
			expect(not allowed.matches("bob"), "a scope allows nothing else")
			with scoped.consume() as consumer:
				expect(consumer.absolute("bob").as_str() == "room/bob", "a scope's consumer shares its root")

	# A broadcast the origin creates is stamped with its path, which its info lends. Each
	# object of it is closed, since an open one keeps it, and so the origin, alive.
	with origin.create_broadcast("room/alice") as publication:
		with publication.broadcast() as broadcast:
			path = broadcast.info().path

		expect(path.as_str() == "room/alice", "a broadcast has its path")

		# Paths own their segments, so Python builds them, and relative ones, from a string.
		try:
			moq.Path.new("/cam/")
			expect(False, "a path with an empty segment is an error")
		except moq.PathInvalidException as e:
			expect(e.error == moq.PathInvalid.EMPTY_SEGMENT, "an empty segment is its own error")
		cam = moq.Path.new("cam")
		expect(cam.as_str() == "cam", "a path builds from its segments")
		joined = path.join(cam)
		expect(joined.as_str() == "room/alice/cam", "paths join")
		expect(joined.strip_prefix(path) == cam, "a path strips its prefix")
		expect(path.strip_prefix(joined) is None, "a path strips no longer prefix")
		expect(joined.resolve(moq.Relative.new("../bob")).as_str() == "room/bob", "a relative path resolves")

	# The driver, run on asyncio's loop until every owner of the origin is gone.
	driving = loop.create_task(drive(driver))
	step(loop)
	# A client publishes the origin through moq-net's generic `with_publisher`, at each
	# type that implements its bound, by reference, so Python keeps its object.
	moq.Client.new().with_publisher_producer(origin).close()
	expect(not driving.done(), "a driver runs while its origin has an owner")
	origin.close()
	# The last owner's close wakes the driver, which then finishes, cleanly.
	try:
		loop.run_until_complete(asyncio.wait_for(driving, 5))
		expect(False, "a driver whose origin is gone finishes")
	except moq.ErrorException as e:
		expect(str(e) == "closed", "a finished driver's error is Rust's")


def catalog(loop):
	# moq-mux reads an OpusHead into hang's catalog config: 44.1 kHz in, stereo, which the
	# catalog lists at the decoder's 48 kHz, keeping the head as its description.
	head = b"OpusHead" + bytes([1, 2, 0x38, 0x01, 0x44, 0xAC, 0, 0, 0, 0, 0])
	audio = moq.codec_opus_audio_config(head)
	expect(audio.sample_rate == 48000, "Opus decodes at 48 kHz")
	expect(audio.channel_count == 2, "the head is stereo")
	expect(audio.codec == moq.CatalogAudioCodec.Opus(), "the codec is Opus")
	expect(audio.description is not None and bytes(audio.description) == head, "the head is the description")
	# A truncated head is moq-mux's error.
	try:
		moq.codec_opus_audio_config(head[:8])
		expect(False, "a truncated head is an error")
	except moq.MuxErrorException as e:
		expect(str(e) != "", "a truncated head's error has a message")

	# A subscription's ranges, from any iterable of them: a list.
	b = moq.Bound
	groups = moq.TrackRange.groups(moq.BoundsU64(moq.BoundU64(b.INCLUDED, 2), moq.BoundU64(b.EXCLUDED, 6)))
	ranged = moq.TrackSubscription.default().with_ranges([moq.TrackRange.live(), groups]).ranges
	expect(ranged == [moq.TrackRange.live(), groups], "a subscription keeps its ranges")
	expect(groups.start == moq.TrackAnchor.Absolute(2), "groups start at their first")
	expect(groups.end == moq.TrackAnchor.Absolute(6), "groups end before their end")

	# A catalog at moq-mux's untyped extension, from the config Python builds, whose generic
	# `is_claimed` is named by the last segment of its argument, and whose `mutate` lends
	# the catalog to Python.
	publisher = moq.BroadcastInfo.new().produce()
	extra = moq.CatalogConfig.default().with_catalog_extra(moq.HangCatalogExtra.default())
	producer = moq.CatalogProducerExtra.new(publisher, extra)
	expect(not producer.is_claimed_binary_config("state"), "an unclaimed name is not claimed")
	read = 0

	def count(lent):
		nonlocal read
		if lent.to_json().startswith("{"):
			read += 1

	producer.mutate(count)
	expect(read == 1, "mutate lends the catalog once")

	# A JSON track the catalog lists, published through moq-mux's generic `json_snapshot` at
	# a JSON document, and a section Python parses from text, set in the lent catalog and
	# read back as text.
	status = producer.json_snapshot_config(
		publisher.create_track("status", None), moq.JsonConfig.default(), moq.JsonOptions.default()
	)
	expect(status.name() == "status", "a JSON snapshot has its track's name")
	expect(producer.is_claimed_json_config("status"), "the JSON snapshot claims its name")
	# A value Python parses is published as the track's latest, which a subscriber reads
	# back as JSON text.
	with moq.Json.parse('{"on": true}') as on:
		status.update(on)

	async def latest():
		group = await status.consume().recv_group()
		return None if group is None else await group.read_frame()

	frame = loop.run_until_complete(latest())
	expect(frame is not None and text(frame.payload) == '{"on":true}', "a subscriber reads the latest value")

	def section(lent):
		sections = lent.ext
		sections.set("app", moq.Json.parse('{"v": [1, true]}'))
		lent.set_ext(sections)

	producer.mutate(section)
	app = None

	def back(lent):
		nonlocal app
		expect('"status"' in lent.to_json(), "the catalog lists the JSON track")
		value = lent.ext.get("app")
		app = None if value is None else value.to_text()

	producer.mutate(back)
	expect(app == '{"v":[1,true]}', "the catalog reads its section back as text")
	# Malformed text fails to parse, and a reserved section is moq-mux's error.
	try:
		moq.Json.parse('{"v":').close()
		expect(False, "malformed JSON fails to parse")
	except moq.StringException as e:
		expect(e.error == "EOF while parsing a value at line 1 column 5", "a parse error is serde_json's")
	try:
		with moq.CatalogHangExtra.default() as hang:
			hang.set("json", moq.Json.parse("1"))
		expect(False, "a reserved section is an error")
	except moq.MuxErrorException as e:
		expect(str(e) == "reserved catalog section: json", "a reserved section is moq-mux's error")


def config():
	# moq-tokio's terminal connect errors, which moq-net's `Error` shares no names with.
	expect(moq.ConnectError.UNAUTHORIZED.is_auth(), "unauthorized is an auth error")
	expect(str(moq.ConnectError.UNAUTHORIZED) == "unauthorized", "a connect error prints as Rust's")

	# Session config's foreign types. A URL is parsed from text, failing with why.
	try:
		moq.Url.parse("relay.example").close()
		expect(False, "a relative URL is an error")
	except moq.StringException as e:
		expect(str(e) == "relative URL without a base", "a URL's parse error is Rust's")
	# A session's config, its URL and pinned fingerprints set as Rust assigns them.
	connect = moq.ConnectConfig.default()
	with connect.tls as tls:
		tls.set_fingerprint(["ab:cd"])
		connect.set_tls(tls)
	connect.set_url(moq.Url.parse("https://relay.example:4443/demo"))
	with connect.tls as tls:
		expect(tls.fingerprint == ["ab:cd"], "a TLS config keeps its fingerprints")
	url = connect.url
	connect.close()
	versions = moq.Versions.default()
	with moq.AuthTokens.new([]) as tokens, tokens.dial(url, versions) as dial, dial.url as dialed:
		expect(dialed == url, "a dial keeps its URL")
		expect(str(dialed) == "https://relay.example:4443/demo", "a URL prints as Rust's")
	url.close()
	# A `Vec<String>`, as a list.
	expect(len(moq.ALPNS) > 0, "a constant slice crosses as a list")
	alpns = versions.alpns()
	expect(len(alpns) > 0 and all(alpns), "each version has an ALPN")
	versions.close()
	# A socket address, crossing in a listener's bind.
	bind = moq.ListenBind.Addr(("::1", 4443, 0, 0))
	expect(str(bind) == "[::1]:4443", "an IPv6 bind prints as Rust's")
	v4 = moq.ListenBind.Addr(("127.0.0.1", 0))
	expect(str(v4) == "127.0.0.1:0", "an IPv4 bind prints as Rust's")
	with moq.ListenConfig.default() as listen:
		listen.set_bind(bind)
		# The preferred address of each family, which takes only that family.
		listen.set_preferred_v4(v4._0)
		expect(listen.preferred_v4 == v4._0 and listen.preferred_v6 is None, "a listener keeps its preferred IPv4 address")
		expect(listen.bind == bind, "a listener's config keeps its bind")
	# TLS files, as OS paths, and fingerprints, as strings: none by default.
	with moq.TlsConnect.default() as tls:
		expect(tls.root == [] and tls.fingerprint == [], "a default TLS config lists none")


async def session():
	"""A session, as moq.c runs it: a server bound to port 0 reads back the port it got
	and serves with one call, in a task of its own, and a client dials it with one call and
	reads a frame back, each await resuming once moq-tokio's fallback runtime wakes it from
	its threads."""
	served = moq.origin_spawn()
	live = served.create_broadcast("live")
	live.announce(moq.OriginRoute.default())
	live_track = live.broadcast().create_track("video", None)
	live_group = live_track.append_group()
	live_group.write_frame(None, b"hello")
	live_group.finish()

	server_config = moq.ServerConfig.default()
	listen = server_config.listen
	listen.set_bind(moq.ListenBind.Addr(("::", 0, 0, 0)))
	tls = listen.tls
	tls.set_generate(["localhost"])
	listen.set_tls(tls)
	server_config.set_listen(listen)
	server = server_config.init()
	local = server.local_addr()
	expect(local[1] != 0, "a server bound to port 0 reads back the port it got")
	serving = asyncio.create_task(server.serve_publish(served.consume()))

	connect = moq.ConnectConfig.default()
	connect.set_url(moq.Url.parse(f"moqt://localhost:{local[1]}"))
	insecure = connect.tls
	insecure.set_insecure(True)
	connect.set_tls(insecure)
	client_config = moq.ClientConfig.default()
	client_config.set_connect(connect)
	client = client_config.init()
	received = moq.origin_spawn()
	remote = received.consume()
	connection = client.clone().with_reconnect(False).consume(received)
	expect(connection is not None, "a client with a URL dials it")
	established = await connection.established()

	broadcast = await remote.request_broadcast("live", None)
	subscriber = await broadcast.track("video").subscribe(None)
	group = await subscriber.recv_group()
	frame = None if group is None else await group.read_frame()
	expect(frame is not None and text(frame.payload) == "hello", "a session reads the frame the server serves")

	await established.close_()
	await client.close_()
	serving.cancel()
	try:
		await serving
	except asyncio.CancelledError:
		pass


loop = asyncio.new_event_loop()
local(loop)
origin(loop)
catalog(loop)
config()
# A stall fails with the timeout's exception.
loop.run_until_complete(asyncio.wait_for(session(), 30))
loop.close()

if failures == 0:
	print("ok")
sys.exit(0 if failures == 0 else 1)
