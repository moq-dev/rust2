// moq.c's script through the generated C++ header: a broadcast's track, published and
// subscribed to in one process, with every read polled as a future, and an origin, with
// moq-pattern, kio, moq-tokio, hang, and moq-mux exported into the same library,
// the catalog lent to C++ for a call, and a session's config filled in field by field.
//
// The local reads need no runtime: the waker counts wakes, and each poll that must wait
// is polled again after the write that wakes it. A session over QUIC runs on moq-tokio's
// fallback runtime, whose threads wake C++'s.

#include <atomic>
#include <chrono>
#include <condition_variable>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <deque>
#include <functional>
#include <mutex>
#include <string>
#include <thread>

#include "moq.hpp"

namespace {

int failures = 0;

#define CHECK(cond)                                                                                                    \
	do {                                                                                                               \
		if (!(cond)) {                                                                                                 \
			std::fprintf(stderr, "%s:%d: %s\n", __FILE__, __LINE__, #cond);                                            \
			failures++;                                                                                                \
		}                                                                                                              \
	} while (0)

// Whether a returned buffer holds `s`.
bool is(const moq::buf &buf, const char *s) {
	bool same = buf.str() == s;
	if (!same) {
		std::fprintf(stderr, "got \"%.*s\", want \"%s\"\n", static_cast<int>(buf.size()),
		             reinterpret_cast<const char *>(buf.data()), s);
	}
	return same;
}

int wakes = 0;

// Runs the jobs an executor is given on a thread of its own, until destroyed.
class worker {
	std::mutex mutex_;
	std::condition_variable cv_;
	std::deque<std::function<void()>> jobs_;
	bool stopping_ = false;
	std::thread thread_;

	void run() {
		for (;;) {
			std::function<void()> job;
			{
				std::unique_lock<std::mutex> lock(mutex_);
				cv_.wait(lock, [&]() { return stopping_ || !jobs_.empty(); });
				if (jobs_.empty()) {
					return;
				}
				job = std::move(jobs_.front());
				jobs_.pop_front();
			}
			job();
		}
	}

public:
	worker() : thread_([this]() { run(); }) {}
	~worker() {
		{
			std::lock_guard<std::mutex> lock(mutex_);
			stopping_ = true;
		}
		cv_.notify_one();
		thread_.join();
	}

	auto executor() {
		return [this](std::function<void()> job) {
			{
				std::lock_guard<std::mutex> lock(mutex_);
				jobs_.push_back(std::move(job));
			}
			cv_.notify_one();
		};
	}
};

// Fails the run rather than hanging when what it guards stalls past `limit`.
class watchdog {
	std::mutex mutex_;
	std::condition_variable cv_;
	bool done_ = false;
	std::thread thread_;

public:
	explicit watchdog(std::chrono::seconds limit)
	    : thread_([this, limit]() {
		      std::unique_lock<std::mutex> lock(mutex_);
		      if (!cv_.wait_for(lock, limit, [&]() { return done_; })) {
			      std::fprintf(stderr, "stalled for %lld seconds\n", static_cast<long long>(limit.count()));
			      std::abort();
		      }
	      }) {}
	~watchdog() {
		{
			std::lock_guard<std::mutex> lock(mutex_);
			done_ = true;
		}
		cv_.notify_one();
		thread_.join();
	}
};

} // namespace

int main() {
	moq::waker waker([]() { wakes++; });

	moq::broadcast_producer broadcast = moq::broadcast_info::new_().produce();
	auto track = broadcast.create_track("video", std::nullopt);
	CHECK(track && is(track->name(), "video"));

	// A second track of the same name is an error, printed as Rust prints it.
	auto duplicate = broadcast.create_track("video", std::nullopt);
	CHECK(!duplicate && is(duplicate.error().to_string(), "duplicate"));

	moq::track_consumer consumer = track->consume();
	auto subscribed = consumer.subscribe(std::nullopt).poll(waker);
	CHECK(subscribed && *subscribed);
	moq::track_subscriber subscriber = std::move(**subscribed);

	// A group's arrival wakes the subscriber's pending receive.
	auto receiving = subscriber.recv_group();
	CHECK(!receiving.poll(waker));
	auto writer = track->append_group();
	CHECK(writer && wakes == 1);
	auto received = receiving.poll(waker);
	CHECK(received && *received && received->value());
	moq::group_consumer group = std::move(*received->value());
	CHECK(group.index() == 0);

	// So does a frame's.
	{
		auto reading = group.read_frame();
		CHECK(!reading.poll(waker));
		CHECK(writer->write_frame(nullptr, "hello"));
		CHECK(wakes == 2);
		auto frame = reading.poll(waker);
		CHECK(frame && *frame && frame->value());
		const moq::frame_frame &hello = *frame->value();
		CHECK(is(hello.payload(), "hello"));
		CHECK(!hello.timestamp());
		// When it arrived: kio's clock's `Instant`, from kio's own export.
		CHECK(moq::clock_now().checked_duration_since(hello.arrived()).has_value());
	}

	// A timestamp is a `Copy` value behind a handle: passing it copies it out. The track is
	// untimed, so a timestamped frame is refused.
	auto at = moq::timestamp::from_millis(1500);
	CHECK(at && at->as_secs() == 1);
	auto late = writer->write_frame(&*at, "late");
	CHECK(!late && is(late.error().to_string(), "frame timestamp doesn't fit its track"));
	CHECK(at->as_secs() == 1);

	// The last frame, bytes that are not UTF-8, then the group's end, which reads as None,
	// both blocking.
	CHECK(writer->write_frame(nullptr, "\xffworld"));
	CHECK(std::move(*writer).finish());
	auto world = group.read_frame().get();
	CHECK(world && world.value() && is(world.value()->payload(), "\xffworld"));
	auto end = group.read_frame().get();
	CHECK(end && !end.value());

	// An origin and the driver that runs it, a tuple. Its root, which it lends, is returned
	// as a new handle to a clone.
	auto [origin, driver] = moq::origin_producer::new_(moq::origin_config::default_());
	CHECK(is(origin.root().as_str(), ""));

	// Scoped beneath a root to patterns, which moq-pattern's own export builds.
	{
		auto alice = moq::pattern::subtree("alice");
		CHECK(alice);
		moq::patterns patterns = moq::patterns::new_();
		CHECK(patterns.insert(*alice));
		auto scoped = origin.scope("room", patterns);
		CHECK(scoped);
		CHECK(is(scoped->root().as_str(), "room"));
		auto absolute = scoped->absolute("alice");
		CHECK(absolute && is(absolute->as_str(), "room/alice"));
		moq::patterns allowed = scoped->allowed();
		CHECK(allowed.matches("alice/cam"));
		CHECK(!allowed.matches("bob"));
		absolute = scoped->consume().absolute("bob");
		CHECK(absolute && is(absolute->as_str(), "room/bob"));
	}

	// A broadcast the origin creates is stamped with its path, which its info lends.
	{
		auto publication = origin.create_broadcast("room/alice");
		CHECK(publication);
		moq::path path = publication->broadcast().info().path();
		CHECK(is(path.as_str(), "room/alice"));

		// Paths own their segments, so C++ builds them, and relative ones, from a string.
		auto empty = moq::path::new_("/cam/");
		CHECK(!empty && empty.error() == moq::path_error::empty_segment);
		auto cam = moq::path::new_("cam");
		CHECK(cam && is(cam->as_str(), "cam"));
		auto joined = path.join(*cam);
		CHECK(joined && is(joined->as_str(), "room/alice/cam"));
		std::optional<moq::path> stripped = joined->strip_prefix(path);
		CHECK(stripped && *stripped == *cam);
		CHECK(!path.strip_prefix(*joined));
		auto up = moq::relative::new_("../bob");
		CHECK(up);
		auto resolved = joined->resolve(*up);
		CHECK(resolved && is(resolved->as_str(), "room/bob"));
	}

	// The driver, polled as a language's event loop would: at the time, and with a waiter
	// made from the waker and held across polls, since what a poll registers lives only as
	// long as its waiter. It runs until every owner of the origin is gone.
	moq::waiter waiter = moq::waiter::new_(waker);
	CHECK(driver.poll(moq::clock_now(), waiter));
	{
		// A client publishes the origin through moq-net's generic `with_publisher`, at each
		// type that implements its bound, by reference, so C++ keeps its handle.
		moq::client publishing = moq::client::new_().with_publisher_producer(origin);
		(void)publishing;
	}
	int before = wakes;
	{
		moq::origin_producer last = std::move(origin);
	}
	// The last owner's drop wakes the driver, which then finishes, cleanly.
	CHECK(wakes > before);
	auto closed = driver.poll(moq::clock_now(), waiter);
	CHECK(!closed && is(closed.error().to_string(), "closed"));

	// moq-mux reads an OpusHead into hang's catalog config: 44.1 kHz in, stereo, which the
	// catalog lists at the decoder's 48 kHz, keeping the head as its description.
	static const std::uint8_t head[] = {'O', 'p', 'u', 's', 'H', 'e', 'a', 'd', 1, 2, 0x38, 0x01, 0x44, 0xac, 0, 0, 0, 0, 0};
	auto audio = moq::codec_opus_audio_config(moq::view(head, sizeof head));
	CHECK(audio);
	CHECK(audio->sample_rate() == 48000);
	CHECK(audio->channel_count() == 2);
	CHECK(std::holds_alternative<moq::catalog_audio_codec_opus>(audio->codec()));
	auto description = audio->description();
	CHECK(description && description->size() == sizeof head && std::memcmp(description->data(), head, sizeof head) == 0);
	// A truncated head is moq-mux's error.
	auto truncated = moq::codec_opus_audio_config(moq::view(head, 8));
	CHECK(!truncated && !truncated.error().to_string().empty());

	// A subscription's ranges, from any iterable of them: a vector.
	{
		moq::track_range live = moq::track_range::live();
		moq::track_subscription ranged = moq::track_subscription::default_().with_ranges(
		    {live, moq::track_range::groups({{MOQ_BOUND_INCLUDED, 2}, {MOQ_BOUND_EXCLUDED, 6}})});
		std::vector<moq::track_range> got = ranged.ranges();
		CHECK(got.size() == 2 && got[0] == live);
		CHECK(got.size() == 2 && std::get<moq::track_anchor_absolute>(got[1].start)._0 == 2);
		CHECK(got.size() == 2 && got[1].end && std::get<moq::track_anchor_absolute>(*got[1].end)._0 == 6);
	}

	// A catalog at moq-mux's untyped extension, from the config C++ builds, whose generic
	// `is_claimed` is named by the last segment of its argument, and whose `mutate` lends
	// the catalog to C++.
	{
		moq::broadcast_producer publisher = moq::broadcast_info::new_().produce();
		moq::catalog_config_extra extra =
		    moq::catalog_config::default_().with_catalog_extra(moq::hang_catalog_extra::default_());
		auto catalog = moq::catalog_producer_extra::new_(publisher, std::move(extra));
		CHECK(catalog);
		CHECK(!catalog->is_claimed_binary_config("state"));
		int read_catalog = 0;
		CHECK(catalog->mutate([&](moq::hang_catalog_extra &lent) {
			auto json = lent.to_json();
			CHECK(json);
			if (json && json.value().str().substr(0, 1) == "{") {
				read_catalog++;
			}
		}));
		CHECK(read_catalog == 1);

		// A JSON track the catalog lists, published through moq-mux's generic
		// `json_snapshot` at a JSON document, and a section C++ parses from text, set in the
		// lent catalog and read back as text.
		auto status = publisher.create_track("status", std::nullopt);
		CHECK(status);
		auto status_snapshot = catalog->json_snapshot_config(std::move(*status), moq::json_config::default_(),
		                                                     moq::json_options::default_());
		CHECK(status_snapshot && is(status_snapshot->name(), "status"));
		CHECK(catalog->is_claimed_json_config("status"));
		// A value C++ parses is published as the track's latest, which a subscriber reads
		// back as JSON text.
		CHECK(status_snapshot->update(moq::json::parse(R"({"on": true})").value()));
		moq::track_subscriber status_subscriber = status_snapshot->consume();
		auto status_group = status_subscriber.recv_group().get();
		CHECK(status_group && status_group.value());
		if (status_group && status_group.value()) {
			auto status_frame = status_group.value()->read_frame().get();
			CHECK(status_frame && status_frame.value() && is(status_frame.value()->payload(), R"({"on":true})"));
		}
		auto app = moq::json::parse(R"({"v": [1, true]})");
		CHECK(app);
		CHECK(catalog->mutate([&](moq::hang_catalog_extra &lent) {
			moq::catalog_hang_extra sections = lent.ext();
			CHECK(sections.set("app", std::move(*app)));
			lent.set_ext(std::move(sections));
		}));
		CHECK(catalog->mutate([&](moq::hang_catalog_extra &lent) {
			auto json = lent.to_json();
			CHECK(json && json.value().str().find("\"status\"") != std::string_view::npos);
			auto section = lent.ext().get("app");
			CHECK(section && is(section->to_text(), R"({"v":[1,true]})"));
		}));
		// Malformed text fails to parse, and a reserved section is moq-mux's error.
		auto broken = moq::json::parse(R"({"v":)");
		CHECK(!broken && is(broken.error(), "EOF while parsing a value at line 1 column 5"));
		moq::catalog_hang_extra sections = moq::catalog_hang_extra::default_();
		auto reserved = sections.set("json", moq::json::parse("1").value());
		CHECK(!reserved && is(reserved.error().to_string(), "reserved catalog section: json"));
	}

	// A session's config, filled in as Rust assigns it, and a nested field by getting a copy,
	// editing it, and setting it back.
	{
		moq::connect_config connect = moq::connect_config::default_();
		moq::tls_connect tls = connect.tls();
		tls.set_insecure(true);
		tls.set_host_name("relay.example");
		tls.set_fingerprint({"ab:cd"});
		connect.set_tls(std::move(tls));
		connect.set_implementation("moq.cpp");
		connect.set_url(moq::url::parse("https://relay.example:4443/demo").value());
		CHECK(connect.tls().insecure() == std::optional<bool>(true));
		CHECK(is(*connect.tls().host_name(), "relay.example"));
		auto fingerprints = connect.tls().fingerprint();
		CHECK(fingerprints.size() == 1 && is(fingerprints[0], "ab:cd"));
		CHECK(is(*connect.implementation(), "moq.cpp"));
		CHECK(is(connect.url()->to_string(), "https://relay.example:4443/demo"));

		// A listener's bind, a socket address.
		moq::socket_addr addr{};
		addr.ip[15] = 1;
		addr.port = 4443;
		addr.v6 = true;
		moq::listen_config listen = moq::listen_config::default_();
		listen.set_bind(moq::listen_bind(moq::listen_bind_addr{addr}));
		auto bind = listen.bind();
		CHECK(bind && is(moq::listen_bind_to_string(*bind), "[::1]:4443"));
		// The preferred address of each family, which takes only that family.
		listen.set_preferred_v6(addr);
		auto preferred = listen.preferred_v6();
		CHECK(preferred && preferred->v6 && preferred->port == 4443 && !listen.preferred_v4());
		// A constant slice, as a vector of clones.
		CHECK(!moq::alpns().empty());
	}

	// moq-tokio's terminal connect errors, which moq-net's `Error` shares no names with.
	CHECK(moq::connect_error_is_auth(moq::connect_error::unauthorized));
	CHECK(is(moq::connect_error_to_string(moq::connect_error::unauthorized), "unauthorized"));

	// A session, as moq.c runs it: a server bound to port 0 reads back the port it got and
	// serves with one call, on an executor of its own, and a client dials it with one call
	// and reads a frame back, blocking on each read while moq-tokio's fallback runtime wakes
	// it from its threads.
	{
		watchdog limit(std::chrono::seconds(30));
		moq::origin_producer served = moq::origin_spawn();
		auto live = served.create_broadcast("live");
		CHECK(live && live->announce(moq::origin_route::default_()));
		moq::broadcast_producer live_broadcast = live->broadcast();
		auto live_track = live_broadcast.create_track("video", std::nullopt);
		CHECK(live_track);
		auto live_group = live_track->append_group();
		CHECK(live_group && live_group->write_frame(nullptr, "hello"));
		CHECK(std::move(*live_group).finish());

		moq::server_config server_config = moq::server_config::default_();
		moq::listen_config listen = server_config.listen();
		moq::socket_addr any{};
		any.v6 = true;
		listen.set_bind(moq::listen_bind(moq::listen_bind_addr{any}));
		moq::tls_listen tls = listen.tls();
		tls.set_generate({"localhost"});
		listen.set_tls(std::move(tls));
		server_config.set_listen(std::move(listen));
		auto server = std::move(server_config).init();
		CHECK(server);
		auto local = server->local_addr();
		CHECK(local && local->v6 && local->port != 0);
		worker serving_thread;
		std::atomic<bool> stopped{false};
		moq::task serving = std::move(*server).serve_publish(served.consume()).then(serving_thread.executor(),
		                                                                            [&](auto &&) { stopped = true; });

		moq::connect_config connect = moq::connect_config::default_();
		connect.set_url(moq::url::parse("moqt://localhost:" + std::to_string(local->port)).value());
		moq::tls_connect insecure = connect.tls();
		insecure.set_insecure(true);
		connect.set_tls(std::move(insecure));
		moq::client_config client_config = moq::client_config::default_();
		client_config.set_connect(std::move(connect));
		auto client = std::move(client_config).init();
		CHECK(client);
		moq::origin_producer incoming = moq::origin_spawn();
		moq::origin_consumer remote = incoming.consume();
		auto connection = moq::tokio_client(*client).with_reconnect(false).consume(std::move(incoming));
		CHECK(connection);
		auto established = std::move(*connection).established().get();
		CHECK(established);

		auto remote_broadcast = remote.request("live", std::nullopt).get();
		CHECK(remote_broadcast);
		auto remote_track = remote_broadcast->track("video");
		CHECK(remote_track);
		auto remote_subscriber = remote_track->subscribe(std::nullopt).get();
		CHECK(remote_subscriber);
		auto remote_group = remote_subscriber->recv_group().get();
		CHECK(remote_group && remote_group.value());
		auto remote_frame = remote_group.value()->read_frame().get();
		CHECK(remote_frame && remote_frame.value() && is(remote_frame.value()->payload(), "hello"));

		CHECK(std::move(*established).close().get());
		std::move(*client).close().get();
		serving = moq::task();
		CHECK(!stopped);
	}

	if (failures == 0) {
		std::printf("ok\n");
	}
	return failures != 0;
}
