// The api fixture's C++ test: the script fixtures/api/examples/transcript.rs runs
// natively, through the generated C++ header. tests/fixtures.rs compares the transcripts.
//
// With an argument, it instead checks the futures' semantics, or breaks a contract of the
// ABI, which must abort.

#include <chrono>
#include <condition_variable>
#include <cstdio>
#include <cstring>
#include <deque>
#include <functional>
#include <mutex>
#include <string>
#include <thread>
#include <vector>

#include "api.hpp"

#ifdef _WIN32
#include <fcntl.h>
#include <io.h>
#endif

namespace {

// Print a returned buffer.
void print(const char *before, const api::buf &buf, const char *after) {
	std::printf("%s%.*s%s", before, static_cast<int>(buf.size()), reinterpret_cast<const char *>(buf.data()), after);
}

// Bytes as Rust's `escape_ascii` prints those of a path.
void print_escaped(const char *before, const api::buf &buf, const char *after) {
	std::printf("%s", before);
	for (std::uint8_t b : buf) {
		switch (b) {
		case '\t':
			std::printf("\\t");
			break;
		case '\r':
			std::printf("\\r");
			break;
		case '\n':
			std::printf("\\n");
			break;
		case '\\':
		case '\'':
		case '"':
			std::printf("\\%c", b);
			break;
		default:
			if (b >= 0x20 && b < 0x7f) {
				std::putchar(b);
			} else {
				std::printf("\\x%02x", b);
			}
		}
	}
	std::printf("%s", after);
}

// Strings as Rust's Debug prints a list of them.
void print_strings(const std::vector<api::buf> &strings) {
	std::printf("[");
	for (std::size_t i = 0; i < strings.size(); i++) {
		print(i == 0 ? "\"" : ", \"", strings[i], "\"");
	}
	std::printf("]");
}

int wakes = 0;

// The Log callback: keeps even levels.
class print_log final : public api::log {
public:
	bool log_(std::uint8_t level, api::view line) const override {
		std::printf("log %u %.*s\n", level, static_cast<int>(line.size()), reinterpret_cast<const char *>(line.data()));
		return level % 2 == 0;
	}
	~print_log() override {
		std::printf("freed\n");
	}
};

// A bound, as Rust's Debug prints it.
void print_bound(api::bound_u64 bound) {
	switch (bound.kind) {
	case API_BOUND_INCLUDED:
		std::printf("Included(%llu)", static_cast<unsigned long long>(bound.value));
		break;
	case API_BOUND_EXCLUDED:
		std::printf("Excluded(%llu)", static_cast<unsigned long long>(bound.value));
		break;
	case API_BOUND_UNBOUNDED:
		std::printf("Unbounded");
		break;
	}
}

const char *yes(bool b) {
	return b ? "true" : "false";
}

unsigned long long ull(std::uint64_t v) {
	return static_cast<unsigned long long>(v);
}

// A queue of jobs that a test runs when it chooses: the executor futures run on.
struct queue {
	std::mutex mutex;
	std::deque<std::function<void()>> jobs;

	auto executor() {
		return [this](std::function<void()> job) {
			std::lock_guard<std::mutex> lock(mutex);
			jobs.push_back(std::move(job));
		};
	}

	// Run jobs until none are left.
	void drain() {
		for (;;) {
			std::function<void()> job;
			{
				std::lock_guard<std::mutex> lock(mutex);
				if (jobs.empty()) {
					return;
				}
				job = std::move(jobs.front());
				jobs.pop_front();
			}
			job();
		}
	}
};

int script() {
	std::printf("limit %llu\n", ull(api::limit));

	api::point start{1, 2};
	api::point p = start.offset(2, 3);
	std::printf("offset %d %d\n", p.x, p.y);
	print("debug ", p.debug(), "\n");
	std::printf("eq %s\n", yes(p == api::point{3, 5}));
	api::point d = api::point::default_();
	std::printf("default %d %d\n", d.x, d.y);

	api::counter c = api::counter::new_("c", 5);
	std::printf("get %llu\n", ull(c.get()));
	print("name ", c.name(), "\n");
	if (auto v = c.add(10)) {
		std::printf("add ok %llu\n", ull(*v));
	}
	if (auto v = c.add(2000); !v) {
		print("add err ", api::error_to_string(v.error()), " ");
		print("", api::error_debug(v.error()), "\n");
	}
	if (auto v = c.sub(20); !v) {
		std::printf("sub err underflow by %llu ", ull(std::get<api::error_underflow>(v.error()).by));
		print("", api::error_debug(v.error()), "\n");
	}
	c.rename("d");
	print("name ", c.name(), "\n");
	std::printf("is d %s\n", yes(c.is("d")));
	{
		api::counter copy = c;
		(void)copy.add(1);
		std::printf("clone %llu original %llu\n", ull(copy.get()), ull(c.get()));
	}
	print("debug ", c.debug(), "\n");
	api::label label = std::move(c).into_label(p);
	print("label ", label.debug(), "\n");
	// An empty buffer C++ made is not none.
	std::printf("note %s %s\n", yes(api::has_text(api::note{api::buf()})), yes(api::has_text(api::note{})));

	// A tuple returns through one out pointer per element.
	auto [below, above] = api::counter::new_("s", 7).split(5);
	print("split ", below.name(), " ");
	std::printf("%llu ", ull(below.get()));
	print("", above.name(), " ");
	std::printf("%llu\n", ull(above.get()));
	auto [only] = api::only(api::counter::new_("o", 3));
	print("only ", only.name(), " ");
	std::printf("%llu\n", ull(only.get()));
	auto [w8, w16, w32, w64] = api::widths(98305);
	std::printf("widths %u %d %u %llu\n", w8, w16, w32, ull(w64));
	for (std::uint64_t divisor : {2, 0}) {
		if (auto r = api::divide(7, divisor)) {
			std::printf("divide %llu %llu\n", ull(std::get<0>(*r)), ull(std::get<1>(*r)));
		} else {
			print("divide err ", api::error_to_string(r.error()), "\n");
		}
	}
	// A tuple inside another type is a std::tuple.
	for (std::uint64_t n : {98305, 0}) {
		if (auto spread = api::spread(n)) {
			auto [s8, s16, s32, s64] = *spread;
			std::printf("spread %u %d %u %llu\n", s8, s16, s32, ull(s64));
		} else {
			std::printf("spread none\n");
		}
	}
	for (const char *needle : {"lo", "x"}) {
		if (auto found = api::find("hello", needle)) {
			std::printf("find %zu %zu\n", std::get<0>(*found), std::get<1>(*found));
		} else {
			std::printf("find none\n");
		}
	}
	std::printf("rows");
	auto stat_rows = api::rows({api::point{1, 2}, api::point{3, 3}});
	for (std::size_t i = 0; i < stat_rows.size(); i++) {
		auto &[name, shape, at] = stat_rows[i];
		print(i == 0 ? " " : ", ", name, " ");
		print("", api::shape_debug(shape), " ");
		std::printf("%d %d", at.x, at.y);
	}
	std::printf("\n");
	{
		// Each handle in a tuple moves in, as one taken by value does.
		std::vector<std::tuple<api::view, api::counter>> tallies;
		tallies.emplace_back("low", api::counter::new_("l", 1));
		tallies.emplace_back("high", api::counter::new_("h", 9));
		auto board = api::board::new_(std::move(tallies));
		std::printf("ranked");
		auto ranked = board.ranked();
		for (std::size_t i = 0; i < ranked.size(); i++) {
			auto &[name, counter] = ranked[i];
			print(i == 0 ? " " : ", ", name, " ");
			print("", counter.name(), " ");
			std::printf("%llu", ull(counter.get()));
		}
		std::printf("\n");
	}
	api::span span{{2, 10}, std::make_tuple(api::buf("s"), api::shape::square)};
	auto [lower_half, upper_half] = api::halves(span);
	std::printf("halves %u %u %u %u ", std::get<0>(lower_half), std::get<1>(lower_half), std::get<0>(upper_half),
				std::get<1>(upper_half));
	print("", span.debug(), "\n");
	auto [swapped_n, swapped_text] = api::swapped({"t", 4});
	std::printf("swapped %u ", swapped_n);
	print("", swapped_text, "\n");
	using single = std::tuple<std::optional<std::uint32_t>>;
	for (const auto &value : {std::optional<single>(), std::optional<single>(single()), std::optional<single>(single(3))}) {
		auto doubled = api::doubled(value);
		if (!doubled) {
			std::printf("doubled None\n");
		} else if (auto n = std::get<0>(*doubled)) {
			std::printf("doubled Some((Some(%u),))\n", *n);
		} else {
			std::printf("doubled Some((None,))\n");
		}
	}
	// A borrowed or boxed tuple of one is a tuple, not its element.
	{
		std::uint8_t lent_bytes[] = {1, 2};
		std::uint8_t boxed_bytes[] = {3};
		auto [lent] = api::lent_back(std::make_tuple(api::view(lent_bytes, 2)));
		auto [boxed] = api::boxed_tuple(api::view(boxed_bytes, 1));
		const api::buf *both[] = {&lent, &boxed};
		for (int i = 0; i < 2; i++) {
			std::printf("%s[", i == 0 ? "lent back " : " boxed ");
			for (std::size_t j = 0; j < both[i]->size(); j++) {
				std::printf("%s%u", j == 0 ? "" : ", ", both[i]->data()[j]);
			}
			std::printf("]");
		}
		std::printf("\n");
	}
	// A borrowed string and an owned one share a tuple struct.
	print("title Some((\"", std::get<0>(*api::title(true)), "\",)) ");
	std::printf("%s ", api::title(false) ? "Some" : "None");
	print("named Some((\"", std::get<0>(*api::named("x")), "\",))\n");

	// Each lent counter is a new handle to a clone.
	api::registry registry = api::registry::new_(api::counter::new_("first", 1));
	std::printf("last %s\n", yes(registry.last().has_value()));
	registry.push(api::counter::new_("last", 2));
	{
		api::counter first = registry.first();
		(void)first.add(10);
		std::printf("first %llu lent %llu\n", ull(registry.first().get()), ull(first.get()));
		api::counter last = *registry.last();
		(void)last.add(10);
		std::printf("last %llu lent %llu\n", ull(registry.last()->get()), ull(last.get()));
	}
	// A closure lent for the call edits the first counter through its class.
	std::uint64_t by = 5;
	bool edited = registry.edit([&](api::counter &counter) {
		(void)counter.add(by);
		return counter.edited() && counter.get() == 6;
	});
	std::printf("edit %s %llu\n", yes(edited), ull(registry.first().get()));
	// A setter moves an object in, as assignment does.
	registry.set_first(api::counter::new_("second", 3));
	std::printf("set first %llu ", ull(registry.first().get()));
	print("", registry.first().name(), "\n");
	std::printf("walk %llu\n", ull(api::walk(3, [](const api::cursor &cursor) { return cursor.at() * 2; })));
	std::printf("each %u\n", api::each(2, [](std::uint32_t at) { std::printf("visit %u\n", at); }));
	std::printf("offset %u\n", api::offset([] { return 7u; }, 3));
	std::printf("shift %u\n", api::shift([] { return 7u; }, 4));

	// A type generic only over lifetimes is a handle to its 'static form.
	{
		api::name name = api::name::owned("a");
		api::name joined = name.join(api::name::owned("b"));
		print("joined ", joined.as_str(), " ");
		print("", joined.debug(), " ");
		std::printf("%s\n", yes(joined == name));
		std::string text = "c";
		joined.rename(text);
		// The name took a copy: the caller's buffer is its own again.
		text[0] = 'x';
		print("renamed ", joined.as_str(), "\n");
		api::name longest = api::longer(std::move(joined), api::name::owned("dd"));
		print("longer ", longest.as_str(), "\n");
		print("unnamed [", api::unnamed().as_str(), "]\n");
	}

	std::printf("total %llu\n", ull(api::total({2, 5})));
	api::range_u64 wide = api::widen({2, 5}, 1);
	std::printf("widen %llu %llu\n", ull(wide.start), ull(wide.end));
	api::bounds_u64 ranges[] = {
	    {{API_BOUND_INCLUDED, 2}, {API_BOUND_EXCLUDED, 5}}, {{API_BOUND_INCLUDED, 2}, {API_BOUND_INCLUDED, 5}},
	    {{API_BOUND_INCLUDED, 3}, {API_BOUND_UNBOUNDED, 0}}, {{API_BOUND_UNBOUNDED, 0}, {API_BOUND_UNBOUNDED, 0}},
	    {{API_BOUND_EXCLUDED, 2}, {API_BOUND_INCLUDED, 4}},
	};
	std::printf("count");
	for (const auto &range : ranges) {
		std::printf(" %zu", api::count(range));
	}
	std::printf("\n");
	std::printf("end ");
	print_bound(api::end(3));
	std::printf(" ");
	print_bound(api::end(0));
	std::printf("\n");
	auto [ends_start, ends_end] = api::ends(3);
	std::printf("ends ");
	print_bound(ends_start);
	std::printf(" ");
	print_bound(ends_end);
	std::printf("\n");
	for (std::uint64_t n : {2, 5}) {
		if (auto r = api::below(n, 3)) {
			std::printf("below %llu\n", ull(*r));
		} else {
			std::printf("below err ");
			print_bound(r.error().start);
			std::printf(" ");
			print_bound(r.error().end);
			std::printf("\n");
		}
	}
	api::window window{{2, 8}, api::end(5)};
	std::printf("window %s %s ", yes(window.contains(4)), yes(window.contains(6)));
	print("", window.debug(), "\n");

	print("", api::greet("x", api::shape::square), "\n");
	std::printf("len %zu\n", api::len("abc"));
	const std::uint8_t binary[] = {0xff, 0x00, 'a'};
	print("hex ", api::hex(api::view(binary, sizeof binary)), "\n");
	print("timed hex ", api::timed_hex(api::view(binary, sizeof binary)), "\n");
	print("sample hex ", api::sample_hex(api::view(binary, sizeof binary)), "\n");
	print("queued hex ", api::queued_hex(api::view(binary, sizeof binary)), "\n");
	print("tick ", api::tick_name(api::label{api::buf("tick"), std::nullopt, std::nullopt}), "\n");
	print("rooted ", api::rooted_name(api::label{api::buf("rooted"), std::nullopt, std::nullopt}), "\n");
	print("stored ", api::stored_text("stored"), "\n");
	print("layer ", api::layer_name(api::label{api::buf("layer"), std::nullopt, std::nullopt}), "\n");
	api::event events[] = {api::event_moved{p}, api::event_named{api::buf("n")}, api::event_done{}};
	for (const auto &event : events) {
		print("", api::describe(event), "\n");
	}

	// A `Copy` handle passed by value is copied, so it stays usable.
	{
		api::stamp stamp = api::stamp::from_millis(2500);
		std::printf("secs %llu\n", ull(stamp.secs()));
		std::printf("later %s\n", yes(api::stamp::from_millis(3000).later(stamp)));
		std::printf("secs %llu\n", ull(stamp.secs()));
		api::message message = api::message::new_(&stamp, "body");
		print("payload ", message.payload(), "\n");
		print("stamp ", message.stamp()->debug(), "\n");
		std::printf("sent %s\n", yes(message.sent()));
		// Setters assign in place: the bytes are copied in, and a `Copy` handle copied.
		api::stamp reset = api::stamp::from_millis(4000);
		message.set_stamp(&reset);
		message.set_payload("edited");
		print("payload ", message.payload(), "\n");
		print("stamp ", message.stamp()->debug(), "\n");
		std::printf("secs %llu\n", ull(reset.secs()));
		message.set_stamp(nullptr);
		std::printf("stamp %s\n", yes(message.stamp().has_value()));
	}

	std::uint8_t levels[] = {1, 2, 3, 4};
	std::printf("kept %u\n", api::emit(std::make_unique<print_log>(), api::view(levels, sizeof levels)));
	{
		api::logger logger = api::logger::new_(std::make_unique<print_log>());
		std::printf("write %s\n", yes(logger.write("hi")));
	}
	std::printf("dropped\n");
	// Rust calls it on another thread, before the call returns.
	std::uint8_t threshold = 2;
	std::printf("filtered %u\n", api::filter(api::view(levels, sizeof levels), [threshold](std::uint8_t level, api::view line) {
		std::printf("filter %u %.*s\n", level, static_cast<int>(line.size()), reinterpret_cast<const char *>(line.data()));
		return level > threshold;
	}));

	{
		api::channel channel = api::channel::new_();
		api::waker waker([]() { wakes++; });
		{
			api::channel_recv_future recv = channel.recv();
			std::printf("recv pending %s\n", yes(!recv.poll(waker).has_value()));
			channel.send("hi");
			std::printf("woken %d\n", wakes);
			if (auto payload = recv.poll(waker)) {
				print("recv ", *payload, "\n");
			}
		}
		channel.send("");
		{
			api::channel_recv_nonempty_future nonempty = channel.recv_nonempty();
			if (auto r = nonempty.poll(waker)) {
				if (*r) {
					std::printf("nonempty %zu\n", r->value().size());
				} else {
					print("nonempty err ", api::error_to_string(r->error()), "\n");
				}
			} else {
				std::printf("nonempty pending\n");
			}
		}

		// The send happens at the call: the future is dropped unpolled.
		channel.send_later("eager");
		{
			api::channel_recv_future recv = channel.recv();
			if (auto payload = recv.poll(waker)) {
				print("recv ", *payload, "\n");
			}
		}
		channel.send("sized");
		{
			api::channel_recv_sized_future sized = channel.recv_sized();
			if (auto r = sized.poll(waker)) {
				print("sized ", std::get<0>(*r), " ");
				std::printf("%zu\n", std::get<1>(*r));
			}
		}
		// The waker moves into Rust, which keeps it until the send wakes it.
		channel.notify(waker);
		channel.send("rung");
		std::printf("woken %d\n", wakes);
	}

	// A value that holds itself is an object.
	{
		api::node list = api::node::new_(1, std::nullopt);
		list.push(2);
		std::printf("node %llu\n", ull(list.sum()));
	}

	{
		api::tally tally = api::tally::new_();
		tally.bump();
		std::uint32_t count = tally.bump();
		std::printf("tally %u %s\n", count, yes(tally == tally));
		std::printf("both %u\n", api::bump_both(tally, tally));
		std::printf("peek %u\n", api::peek(tally, [](const api::tally &lent) { return lent.bump(); }));
	}

	// Another export's types, which this export's function takes and returns.
	{
		api::area area = api::grow(api::area::new_({2, 3}), 1);
		api::size size = area.size();
		std::printf("area %u %u %llu\n", size.width, size.height, ull(area.pixels()));
	}

	// Generic types, each at the instances it infers, named by their arguments.
	api::pair_u32 pair{2, 5};
	api::pair_u32 swapped = pair.swap();
	std::printf("pair %u %u sum %u ", swapped.left, swapped.right, pair.sum());
	print("", pair.debug(), " ");
	std::printf("%s\n", yes(pair == swapped));
	std::printf("even %s %s\n", yes(pair.even()), yes(api::pair_u32{7, 7}.even()));
	// An instance renamed alone, beside another of `Pair` that keeps its name.
	api::size_tag sizes = api::tag({1, 2}, 3);
	api::pair_u8_size flipped = sizes.swap();
	std::printf("sized %u %u ", flipped.left, flipped.right.width);
	print("", flipped.debug(), "\n");
	api::pair_u8_size beside = api::pair_u32{258, 9}.beside_size(sizes.left);
	std::printf("beside %u ", beside.left);
	print("", beside.debug(), "\n");
	{
		// At its default, named as the type itself.
		api::stack stack = api::stack::new_();
		const char *names[] = {"a", "b"};
		for (int i = 0; i < 2; i++) {
			stack.push(api::counter::new_(names[i], static_cast<std::uint64_t>(i) + 1));
		}
		std::optional<api::counter> top = stack.pop();
		std::printf("stack %zu Some(%llu)", stack.depth(), ull(top->get()));
		std::printf(" %s", yes(stack.pop().has_value()));
		std::printf(" %s\n", yes(!stack.pop().has_value()));
		// Generic functions and methods, at each implementation of the trait bounding them,
		// by reference, which leaves C++ its handle. Two arguments sharing a last segment
		// name their instances apart by the module of one.
		api::counter from_point = api::counter::seeded_point("p", {-3, 4});
		api::counter from_size = api::counter::from_size("s", {5, 6});
		api::counter from_counter = api::counter::seeded_counter("c", from_point);
		api::counter from_polar = api::counter::seeded_polar_point("q", {1, 2});
		std::printf("seeded %llu %llu %llu %llu past %llu seeds %llu\n", ull(from_point.get()), ull(from_size.get()),
		            ull(from_counter.get()), ull(from_polar.get()), ull(stack.depth_past_point({1, 2})),
		            ull(api::seeds_point_size({2, 3})));
	}

	// A trait implemented for `Point` and `&Point` apart: one instance.
	std::printf("weigh %u\n", api::weigh_point({-1, 2}));
	// Instances that name each other, and one made only by an impl of it.
	std::printf("cross %u outer %u\n", api::cross(api::left_right_u8{api::right_u8{1}}, api::right_left_u8{api::left_u8{2}}),
	            api::outer_inner_u8::new_(4).get());
	// A bound that outlives the call takes a value, and an instance only a constant names.
	std::printf("kept %llu boxed %u\n", ull(api::kept_point({1, -2})), api::boxed().value);
	std::printf("keep %d\n", api::keep_point({1, -2}));
	// Inferred instances taking a lent closure and a foreign implementation, and one only
	// its impl of a trait names.
	api::pair_u32 sides{2, 5};
	std::printf("apply %u\n", api::apply_u32(sides, [](std::uint32_t side) { return side * 3; }));
	std::printf("report %u\n", api::report_u32(sides, std::make_unique<print_log>()));
	std::printf("wrapped %llu\n", ull(api::kept_wrap_u32({4})));
	std::printf("keeps %u\n", api::keeps(std::make_unique<print_log>()).value);

	// Its Drop runs when C++ destroys the handle, not before.
	{
		api::token token = api::token::new_(7);
		std::printf("token %u dropped %u\n", token.id(), api::dropped());
	}
	std::printf("dropped %u\n", api::dropped());

	// A field and the method of its name, each its own member.
	api::retry retry = api::retry::new_(5u);
	std::optional<std::uint32_t> limit = retry.limit_setting();
	std::printf("retry %d %u %u %u\n", limit.has_value(), limit.value_or(0), retry.limit(), retry.delay());
	// The renamed field's setter, named as its getter.
	retry.set_limit_setting(std::nullopt);
	std::printf("retry %u\n", retry.limit());
	std::printf("slot %u\n", api::slot_u8::new_(200).value());

	// Foreign types: a URL parsed from text, socket addresses, OS paths, and `Vec`s.
	api::url base = api::url::parse("https://example.com/a/b").value();
	print("url ", base.to_string(), " host ");
	if (auto host = api::host(base)) {
		print("Some(\"", *host, "\")\n");
	} else {
		std::printf("None\n");
	}
	if (auto resolved = api::resolve(base, "c?d")) {
		print("resolve ", resolved->to_string(), " ");
		std::printf("%s\n", *resolved == base ? "true" : "false");
	} else {
		print("resolve err ", resolved.error(), "\n");
	}
	if (auto parsed = api::url::parse("no scheme")) {
		print("parse ", parsed->to_string(), "\n");
	} else {
		print("parse err ", parsed.error(), "\n");
	}

	api::socket_addr v4{};
	v4.ip[0] = 127;
	v4.ip[3] = 1;
	v4.port = 8080;
	api::socket_addr v6{};
	v6.ip[15] = 1;
	v6.port = 443;
	v6.v6 = true;
	v6.scope_id = 3;
	auto ports = api::ports({v4, v6});
	std::printf("ports [%u, %u]\n", ports[0], ports[1]);
	std::vector<api::buf> names;
	names.emplace_back(api::view("a"));
	names.emplace_back(api::view("b"));
	api::peer peer = api::peer{v6, names}.next("c");
	std::printf("peer Peer { addr: [::%u%%%u]:%u, names: ", peer.addr.ip[15], peer.addr.scope_id, peer.addr.port);
	print_strings(peer.names);
	std::printf(" } ");
	print_strings(peer.aliases());
	std::printf("\n");

	// A path's bytes as the OS holds them, which need not be UTF-8.
	api::buf file = api::file("dir", "f\\'\"\t");
	auto parents = api::parents({file, "/"});
	print_escaped("file ", file, "\n");
	std::printf("parents");
	for (const auto &parent : parents) {
		if (parent) {
			print_escaped(" ", *parent, "");
		} else {
			std::printf(" none");
		}
	}
	std::printf("\n");

	auto words = api::words(" one two  three ");
	std::printf("words %zu ", words.size());
	print_strings(words);
	std::printf("\n");
	print("sentence ", api::sentence(std::vector<api::view>(words.begin(), words.end())), " ");
	print("", api::sentence(std::nullopt), "\n");
	print("sentence ", api::sentence(std::vector<api::view>()), "|\n");

	std::printf("chunks [");
	auto chunks = api::chunks("abcde", 2);
	for (std::size_t i = 0; i < chunks.size(); i++) {
		std::printf("%s[", i == 0 ? "" : ", ");
		for (std::size_t j = 0; j < chunks[i].size(); j++) {
			std::printf("%s%u", j == 0 ? "" : ", ", chunks[i].data()[j]);
		}
		std::printf("]");
	}
	std::printf("]\n");

	auto counters = api::counters({"x", "y"});
	std::printf("counters");
	for (std::size_t i = 0; i < counters.size(); i++) {
		print(i == 0 ? " " : ", ", counters[i].name(), " ");
		std::printf("%llu", static_cast<unsigned long long>(counters[i].get()));
	}
	std::printf("\n");
	std::vector<api::counter> some, boxed;
	some.push_back(std::move(counters[0]));
	boxed.push_back(std::move(counters[1]));
	auto worth = api::worth(std::move(some), std::move(boxed));
	std::printf("worth %llu %llu\n", ull(worth), ull(api::worth(std::nullopt, {})));

	auto sums = api::sums({{1, 2}, {}, {3}});
	std::printf("sums [%u, %u, %u]\n", sums[0], sums[1], sums[2]);
	auto summed = api::summed({{1, 2}, {-4, 6}});
	auto none_summed = api::summed({});
	std::printf("summed %d %d %d %d\n", summed.x, summed.y, none_summed.x, none_summed.y);
	std::printf("longest %zu %zu\n", api::longest({"a", "abc"}), api::longest({}));

	api::json doc = api::json::parse(R"({"b": [1, 2.5, null], "a": "\u00e9"})").value();
	print("json ", api::encode(doc), "\n");
	print("timed text ", api::timed_text(doc), "\n");
	api::json tagged = api::tagged(std::move(doc), "t");
	print("tagged ", tagged.to_text(), "\n");
	if (auto parsed = api::json::parse("{\"a\":")) {
		print("json parse ", parsed->to_text(), "\n");
	} else {
		print("json parse err ", parsed.error(), "\n");
	}

	api::site site = api::site::new_(base);
	print("site ", site.home().to_string(), " ");
	print("", site.home().to_string(), "\n");

	// Slices, passed in and returned as clones, and the socket address of each family.
	auto shapes = api::shape_all();
	std::printf("shapes [");
	for (std::size_t i = 0; i < shapes.size(); i++) {
		print(i == 0 ? "" : ", ", api::shape_debug(shapes[i]), "");
	}
	std::printf("] greetings ");
	print_strings(api::greetings());
	std::printf("\n");
	api::trail trail = api::trail::new_({api::point{1, 2}, api::point{3, 4}}, {"a", "b"});
	auto points = trail.points();
	std::printf("trail [");
	for (std::size_t i = 0; i < points.size(); i++) {
		print(i == 0 ? "" : ", ", points[i].debug(), "");
	}
	auto labels = trail.labels();
	std::printf("] [");
	for (std::size_t i = 0; i < labels.size(); i++) {
		print(i == 0 ? "" : ", ", labels[i].debug(), "");
	}
	std::printf("]\n");
	// Returned iterators, each collected at the call: drained labels are gone after.
	auto iter_points = trail.iter();
	std::printf("iterated [");
	for (std::size_t i = 0; i < iter_points.size(); i++) {
		print(i == 0 ? "" : ", ", iter_points[i].debug(), "");
	}
	std::printf("] ");
	print_strings(trail.names());
	auto iter_bytes = trail.bytes();
	std::printf(" [");
	for (std::size_t i = 0; i < iter_bytes.size(); i++) {
		std::printf("%s%u", i == 0 ? "" : ", ", iter_bytes.data()[i]);
	}
	std::printf("]\n");
	auto drained = trail.drain();
	std::printf("drained [");
	for (std::size_t i = 0; i < drained.size(); i++) {
		print(i == 0 ? "" : ", ", drained[i].debug(), "");
	}
	auto left = trail.labels();
	std::printf("] then [");
	for (std::size_t i = 0; i < left.size(); i++) {
		print(i == 0 ? "" : ", ", left[i].debug(), "");
	}
	std::printf("]\n");
	auto numbered = api::numbered({"p", "q"});
	std::printf("numbered");
	for (std::size_t i = 0; i < numbered.size(); i++) {
		print(i == 0 ? " " : ", ", numbered[i].name(), " ");
		std::printf("%llu", static_cast<unsigned long long>(numbered[i].get()));
	}
	std::printf("\n");
	api::socket_addr at4{};
	at4.ip[0] = 127;
	at4.ip[3] = 1;
	at4.port = 8080;
	api::socket_addr at6{};
	at6.ip[15] = 1;
	at6.port = 443;
	at6.v6 = true;
	at6.scope_id = 3;
	auto [next4, next6] = api::nexts(at4, at6);
	std::printf("nexts %u.%u.%u.%u %u %u %u\n", next4.ip[0], next4.ip[1], next4.ip[2], next4.ip[3], next4.port, next6.port,
				next6.scope_id);

	// A returned `Vec<&Token>` holds clones, which the vector drops.
	api::token twice = api::token::new_(9);
	std::uint32_t before = api::dropped();
	{
		auto tokens = twice.twice();
		if (tokens.size() != 2 || tokens[1].id() != 9) {
			return 1;
		}
	}
	if (api::dropped() != before + 2) {
		return 1;
	}
	return 0;
}

// The futures' semantics beyond the transcript, each checked by an assertion.
int check(bool ok, const char *what) {
	if (!ok) {
		std::fprintf(stderr, "failed: %s\n", what);
		std::exit(1);
	}
	return 0;
}

int futures() {
	api::channel channel = api::channel::new_();

	// Blocking: the send lands from another thread while it waits.
	{
		std::thread sender([&]() { channel.send("blocked"); });
		api::buf payload = channel.recv().get();
		sender.join();
		check(payload.str() == "blocked", "get returns the payload sent");
	}

	// On an executor: polled there, and called back there once ready.
	queue q;
	{
		std::string got;
		api::task task = channel.recv().then(q.executor(), [&](api::buf payload) { got = payload.str(); });
		q.drain();
		check(got.empty(), "then waits for a send");
		channel.send("then");
		check(got.empty(), "then never polls inside the wake");
		q.drain();
		check(got == "then", "then calls back with the payload");
	}

	// A wake that lands while the future polls, before it returns pending, is kept: the
	// future polls again rather than waiting for a wake that already happened.
	{
		std::uint32_t got = 0;
		api::task task = api::yielded(7).then(q.executor(), [&](std::uint32_t value) { got = value; });
		q.drain();
		check(got == 7, "a wake during the poll polls again");
	}

	// Cancelled between its wake and its poll, a receive loses nothing: the payload stays
	// for the next.
	{
		bool called = false;
		{
			api::task task = channel.recv().then(q.executor(), [&](api::buf) { called = true; });
			q.drain();
			channel.send("kept");
		}
		q.drain();
		check(!called, "a cancelled task never calls back");
		auto next = channel.recv();
		auto payload = next.poll(api::waker([]() {}));
		check(payload && payload->str() == "kept", "a cancelled receive loses nothing");
	}

	// Cancelled from its own callback, it finishes.
	{
		api::task task;
		bool called = false;
		channel.send("self");
		task = channel.recv().then(q.executor(), [&](api::buf) {
			called = true;
			task = api::task();
		});
		q.drain();
		check(called, "a task cancelled in its callback finishes");
	}

	// Destroyed while a wake is posting it, a task returns only once that post is done, so
	// its executor can go with it. This executor blocks inside the wake's post until released:
	// a destroy that returns before then has left the executor in use. Seeing that the
	// destroy waits needs a bound, so it is given 100 ms, more than a destroy that does not
	// wait ever takes.
	{
		std::mutex mutex;
		std::condition_variable cv;
		std::deque<std::function<void()>> jobs;
		int posts = 0;
		bool entered = false;
		bool released = false;
		bool destroyed = false;
		auto exec = [&](std::function<void()> job) {
			std::unique_lock<std::mutex> lock(mutex);
			jobs.push_back(std::move(job));
			// The first post is the task's own first poll; the second, the send's wake.
			if (++posts == 2) {
				entered = true;
				cv.notify_all();
				cv.wait(lock, [&]() { return released; });
			}
		};
		api::task task = channel.recv().then(exec, [](api::buf) {});
		std::function<void()> first;
		{
			std::lock_guard<std::mutex> lock(mutex);
			first = std::move(jobs.front());
			jobs.pop_front();
		}
		first();
		std::thread sender([&]() { channel.send("raced"); });
		{
			std::unique_lock<std::mutex> lock(mutex);
			cv.wait(lock, [&]() { return entered; });
		}
		std::thread destroyer([&]() {
			task = api::task();
			std::lock_guard<std::mutex> lock(mutex);
			destroyed = true;
			cv.notify_all();
		});
		bool early = false;
		{
			std::unique_lock<std::mutex> lock(mutex);
			early = cv.wait_for(lock, std::chrono::milliseconds(100), [&]() { return destroyed; });
			released = true;
		}
		cv.notify_all();
		sender.join();
		destroyer.join();
		check(!early, "destroying a task waits for a post under way");
		jobs.clear();
		check(channel.recv().get().str() == "raced", "a task destroyed mid-post loses nothing");
	}

#if defined(__cpp_impl_coroutine) && __cpp_impl_coroutine >= 201902L && __has_include(<coroutine>)
	// Awaited in a coroutine, resumed on the executor.
	{
		struct detached {
			struct promise_type {
				detached get_return_object() {
					return {std::coroutine_handle<promise_type>::from_promise(*this)};
				}
				std::suspend_never initial_suspend() noexcept {
					return {};
				}
				std::suspend_always final_suspend() noexcept {
					return {};
				}
				void return_void() {}
				void unhandled_exception() {
					std::abort();
				}
			};
			std::coroutine_handle<promise_type> handle;
		};
		std::string got;
		auto read = [&](api::channel &ch) -> detached {
			api::buf payload = co_await ch.recv().via(q.executor());
			got = payload.str();
			std::uint32_t value = co_await api::yielded(3).via(q.executor());
			got += std::to_string(value);
		};
		detached coroutine = read(channel);
		channel.send("awaited");
		q.drain();
		check(coroutine.handle.done() && got == "awaited3", "co_await resumes with each output");
		coroutine.handle.destroy();

		// Destroyed while suspended, the coroutine cancels its receive, which loses nothing.
		got.clear();
		detached cancelled = read(channel);
		q.drain();
		channel.send("again");
		cancelled.handle.destroy();
		q.drain();
		check(got.empty(), "a destroyed coroutine never resumes");
		auto payload = channel.recv().get();
		check(payload.str() == "again", "a destroyed coroutine's receive loses nothing");
	}
#endif
	std::printf("futures ok\n");
	return 0;
}

// Each breaks a contract, which must abort with a message naming the function.
int broken(const char *which) {
	if (std::strcmp(which, "moved") == 0) {
		api::counter c = api::counter::new_("c", 0);
		api::label label = std::move(c).into_label(std::nullopt);
		c.get();
	} else if (std::strcmp(which, "value") == 0) {
		api::counter c = api::counter::new_("c", 0);
		c.sub(1).value();
	} else if (std::strcmp(which, "finished") == 0) {
		api::channel channel = api::channel::new_();
		channel.send("x");
		api::channel_recv_future recv = channel.recv();
		api::waker waker([]() {});
		recv.poll(waker);
		recv.poll(waker);
	} else if (std::strcmp(which, "null view") == 0) {
		api::len(api::view(nullptr, 1));
	} else if (std::strcmp(which, "lent moved") == 0) {
		api::registry registry = api::registry::new_(api::counter::new_("c", 0));
		registry.edit([](api::counter &counter) {
			api::label label = std::move(counter).into_label(std::nullopt);
			return false;
		});
	} else if (std::strcmp(which, "lent kept") == 0) {
		static std::optional<api::counter> kept;
		api::registry registry = api::registry::new_(api::counter::new_("c", 0));
		registry.edit([](api::counter &counter) {
			kept.emplace(std::move(counter));
			return false;
		});
	}
	return 0;
}

} // namespace

int main(int argc, char **argv) {
#ifdef _WIN32
	// The transcript's lines end as Rust's do, which text mode would turn into CRLF.
	_setmode(_fileno(stdout), _O_BINARY);
#endif
	if (argc > 1 && std::strcmp(argv[1], "futures") == 0) {
		return futures();
	}
	if (argc > 1) {
		return broken(argv[1]);
	}
	return script();
}
