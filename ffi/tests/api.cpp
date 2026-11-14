// The api fixture's C++ test: the script fixtures/api/examples/transcript.rs runs
// natively, through the generated C++ header. tests/fixtures.rs compares the transcripts.
//
// With an argument, it instead checks the futures' semantics, or breaks a contract of the
// ABI, which must abort.

#include <algorithm>
#include <chrono>
#include <cmath>
#include <condition_variable>
#include <cstdio>
#include <cstring>
#include <deque>
#include <functional>
#include <memory>
#include <mutex>
#include <optional>
#include <string>
#include <thread>
#include <variant>
#include <vector>
#if __has_include(<version>)
#include <version>
#endif
#ifdef __cpp_lib_span
#include <span>
#endif
#ifdef __cpp_lib_ranges
#include <ranges>
#endif

#include "api.hpp"

#ifdef _WIN32
#include <fcntl.h>
#include <io.h>
#endif

#ifdef __linux__
#include <sys/prctl.h>
#endif

namespace {

// Print a returned buffer.
void print(const char *before, const api::buf &buf, const char *after) {
	std::printf("%s%.*s%s", before, static_cast<int>(buf.size()), reinterpret_cast<const char *>(buf.data()), after);
}

// Print a list as Rust's `{:?}` does, each item as `show` spells it.
template <typename T, typename F> static void print_list(const std::vector<T> &list, F show) {
	std::printf("[");
	for (size_t i = 0; i < list.size(); i++) {
		std::printf(i ? ", %s" : "%s", show(list[i]).c_str());
	}
	std::printf("]");
}

static uint64_t f64_bits(double f) {
	uint64_t bits;
	std::memcpy(&bits, &f, sizeof bits);
	return bits;
}

static double f64_from(uint64_t bits) {
	double f;
	std::memcpy(&f, &bits, sizeof f);
	return f;
}

static uint32_t f32_bits(float f) {
	uint32_t bits;
	std::memcpy(&bits, &f, sizeof bits);
	return bits;
}

// Print each of `n`'s lists, its floats as their bits, as transcript.rs does.
static void numbers(const char *what, const api::Numbers &n) {
	auto line = [&](const char *name, const auto &list, auto show) {
		std::printf("%s %s ", what, name);
		print_list(list, show);
		std::printf("\n");
	};
	auto num = [](auto v) { return std::to_string(v); };
	line("u16", n.u16s, num);
	line("u32", n.u32s, num);
	line("u64", n.u64s, num);
	line("i8", n.i8s, [](int8_t v) { return std::to_string(int(v)); });
	line("i16", n.i16s, num);
	line("i32", n.i32s, num);
	line("i64", n.i64s, num);
	line("f32", n.f32s, [](float v) { return std::to_string(f32_bits(v)); });
	line("f64", n.f64s, [](double v) { return std::to_string(f64_bits(v)); });
	if (n.some) {
		std::printf("%s some Some(", what);
		print_list(*n.some, num);
		std::printf(")\n");
	} else {
		std::printf("%s some None\n", what);
	}
}

// A reader of bytes in memory, which Rust reads and seeks.
struct memory {
	std::string bytes;
	std::size_t at = 0;

	api::result<std::size_t, std::string> read(std::uint8_t *buf, std::size_t len) {
		std::size_t n = std::min(len, bytes.size() - at);
		std::memcpy(buf, bytes.data() + at, n);
		at += n;
		return api::result<std::size_t, std::string>::ok(n);
	}
	api::result<std::uint64_t, std::string> seek(std::int64_t offset, std::ios_base::seekdir dir) {
		std::int64_t base = dir == std::ios_base::beg ? 0 : static_cast<std::int64_t>(dir == std::ios_base::cur ? at : bytes.size());
		if (base + offset < 0) {
			return api::result<std::uint64_t, std::string>::err("before the start");
		}
		at = static_cast<std::size_t>(base + offset);
		return api::result<std::uint64_t, std::string>::ok(at);
	}
};

// A reader whose reads fail, as the transcript's: each seek lands at 0.
struct broken_reader {
	api::result<std::size_t, std::string> read(std::uint8_t *, std::size_t) {
		return api::result<std::size_t, std::string>::err("broken");
	}
	api::result<std::uint64_t, std::string> seek(std::int64_t, std::ios_base::seekdir) {
		return api::result<std::uint64_t, std::string>::ok(0);
	}
};

// A reader whose reads fail, which seeks anywhere from its start, as Rust's `Cursor` does.
struct far_reader : broken_reader {
	api::result<std::uint64_t, std::string> seek_start(std::uint64_t offset) {
		return api::result<std::uint64_t, std::string>::ok(offset);
	}
};

// Print what `tail` read of `file`.
template <class R>
void tail(R &file) {
	auto read = api::tail(file, 5);
	if (read) {
		print("tail ", std::get<0>(*read), "");
		std::printf(" %llu\n", static_cast<unsigned long long>(std::get<1>(*read)));
	} else {
		print("tail error ", read.error(), "\n");
	}
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
class print_log final : public api::Log {
public:
	bool log(std::uint8_t level, api::view line) const override {
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
	std::printf("limit %llu\n", ull(api::LIMIT));

	api::Point start{1, 2};
	api::Point p = start.offset(2, 3);
	std::printf("offset %d %d\n", p.x, p.y);
	print("debug ", p.debug(), "\n");
	std::printf("eq %s\n", yes(p == api::Point{3, 5}));
	api::Point d = api::Point::default_();
	std::printf("default %d %d\n", d.x, d.y);

	api::Counter c = api::Counter::new_("c", 5);
	std::printf("get %llu\n", ull(c.get()));
	print("name ", c.name(), "\n");
	if (auto v = c.add(10)) {
		std::printf("add ok %llu\n", ull(*v));
	}
	if (auto v = c.add(2000); !v) {
		print("add err ", api::Error_to_string(v.error()), " ");
		print("", api::Error_debug(v.error()), "\n");
	}
	if (auto v = c.sub(20); !v) {
		std::printf("sub err underflow by %llu ", ull(std::get<api::Error_Underflow>(v.error()).by));
		print("", api::Error_debug(v.error()), "\n");
	}
	c.rename("d");
	print("name ", c.name(), "\n");
	std::printf("is d %s\n", yes(c.is("d")));
	{
		api::Counter copy = c.clone();
		(void)copy.add(1);
		std::printf("clone %llu original %llu\n", ull(copy.get()), ull(c.get()));
	}
	print("debug ", c.debug(), "\n");
	api::Label label = std::move(c).into_label(p);
	print("label ", label.debug(), "\n");
	// An empty buffer C++ made is not none.
	std::printf("note %s %s\n", yes(api::has_text(api::Note{api::buf()})), yes(api::has_text(api::Note{})));

	// A tuple returns through one out pointer per element.
	auto [below, above] = api::Counter::new_("s", 7).split(5);
	print("split ", below.name(), " ");
	std::printf("%llu ", ull(below.get()));
	print("", above.name(), " ");
	std::printf("%llu\n", ull(above.get()));
	auto [only] = api::only(api::Counter::new_("o", 3));
	print("only ", only.name(), " ");
	std::printf("%llu\n", ull(only.get()));
	auto [w8, w16, w32, w64] = api::widths(98305);
	std::printf("widths %u %d %u %llu\n", w8, w16, w32, ull(w64));
	for (std::uint64_t divisor : {2, 0}) {
		if (auto r = api::divide(7, divisor)) {
			std::printf("divide %llu %llu\n", ull(std::get<0>(*r)), ull(std::get<1>(*r)));
		} else {
			print("divide err ", api::Error_to_string(r.error()), "\n");
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
	auto stat_rows = api::rows({api::Point{1, 2}, api::Point{3, 3}});
	for (std::size_t i = 0; i < stat_rows.size(); i++) {
		auto &[name, shape, at] = stat_rows[i];
		print(i == 0 ? " " : ", ", name, " ");
		print("", api::Shape_debug(shape), " ");
		std::printf("%d %d", at.x, at.y);
	}
	std::printf("\n");
	{
		// Each handle in a tuple moves in, as one taken by value does.
		std::vector<std::tuple<api::view, api::Counter>> meters;
		meters.emplace_back("low", api::Counter::new_("l", 1));
		meters.emplace_back("high", api::Counter::new_("h", 9));
		auto board = api::Board::new_(std::move(meters));
		// The counters a board returns build another, each handle moved back in.
		board = api::Board::new_(std::move(board).into_counters());
		std::printf("ranked");
		auto ranked = board.ranked();
		for (std::size_t i = 0; i < ranked.size(); i++) {
			auto &[name, counter] = ranked[i];
			print(i == 0 ? " " : ", ", name, " ");
			print("", counter.name(), " ");
			std::printf("%llu", ull(counter.get()));
		}
		std::printf("\n");
		auto best = board.best();
		print("best ", best.name(), " ");
		std::printf("%llu\n", ull(best.get()));
	}
	{
		// A map crosses as its pairs and a set as its items, which Rust collects in order,
		// each key once, the last kept.
		api::Atlas loose;
		loose.points.emplace_back(api::buf("b"), api::Point{1, 1});
		loose.points.emplace_back(api::buf("ab"), api::Point{2, 3});
		loose.points.emplace_back(api::buf("b"), api::Point{4, 4});
		loose.ids = {3, 1, 3};
		auto atlas = loose.with("c", api::Point{5, 6});
		std::printf("atlas");
		for (std::size_t i = 0; i < atlas.points.size(); i++) {
			auto &[name, at] = atlas.points[i];
			print(i == 0 ? " " : ", ", name, " ");
			std::printf("%d %d", at.x, at.y);
		}
		std::printf("; ids");
		for (auto id : atlas.ids) {
			std::printf(" %u", id);
		}
		std::printf("\n");
	}
	// Another crate's error crosses as its message.
	for (const char *text : {"80", "x"}) {
		if (auto port = api::port(text)) {
			std::printf("port %u\n", unsigned(port.value()));
		} else {
			print("port err ", port.error(), "\n");
		}
	}
	api::Span span{{2, 10}, std::make_tuple(api::buf("s"), api::Shape::square)};
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
	api::Registry registry = api::Registry::new_(api::Counter::new_("first", 1));
	std::printf("last %s\n", yes(registry.last().has_value()));
	registry.push(api::Counter::new_("last", 2));
	{
		api::Counter first = registry.first_get();
		(void)first.add(10);
		std::printf("first %llu lent %llu\n", ull(registry.first_get().get()), ull(first.get()));
		api::Counter last = *registry.last();
		(void)last.add(10);
		std::printf("last %llu lent %llu\n", ull(registry.last()->get()), ull(last.get()));
	}
	// A closure lent for the call edits the first counter through its class.
	std::uint64_t by = 5;
	bool edited = registry.edit([&](api::borrow<api::Counter> counter) {
		(void)counter->add(by);
		return counter->edited() && counter->get() == 6;
	});
	std::printf("edit %s %llu\n", yes(edited), ull(registry.first_get().get()));
	// A setter moves an object in, as assignment does.
	registry.first_set(api::Counter::new_("second", 3));
	std::printf("set first %llu ", ull(registry.first_get().get()));
	print("", registry.first_get().name(), "\n");
	std::printf("walk %llu\n", ull(api::walk(3, [](const api::Cursor &cursor) { return cursor.at() * 2; })));
	std::printf("each %u\n", api::each(2, [](std::uint32_t at) { std::printf("visit %u\n", at); }));
	std::printf("offset %u\n", api::offset([] { return 7u; }, 3));
	std::printf("shift %u\n", api::shift([] { return 7u; }, 4));

	// A type generic only over lifetimes is a handle to its 'static form.
	{
		api::Name name = api::Name::owned("a");
		api::Name joined = name.join(api::Name::owned("b"));
		print("joined ", joined.as_str(), " ");
		print("", joined.debug(), " ");
		std::printf("%s\n", yes(joined == name));
		std::string text = "c";
		joined.rename(text);
		// The name took a copy: the caller's buffer is its own again.
		text[0] = 'x';
		print("renamed ", joined.as_str(), "\n");
		api::Name longest = api::longer(std::move(joined), api::Name::owned("dd"));
		print("longer ", longest.as_str(), "\n");
		print("unnamed [", api::unnamed().as_str(), "]\n");
		api::Name lit = api::Name::borrowed("lit");
		print("borrowed ", lit.as_str(), " ");
		std::printf("%s %s\n", lit.shares(api::Name::borrowed("lit")) ? "true" : "false",
			lit.shares(api::Name::borrowed("other")) ? "true" : "false");
	}

	std::printf("total %llu\n", ull(api::total({2, 5})));
	std::printf("reach %u\n", unsigned(api::reach(2, 5)));
	std::printf("scaled");
	for (auto v : {api::scaled({UINT64_MAX, UINT64_MAX}, 1), api::scaled({1, 1}, 1ULL << 63), api::scaled({0, 1ULL << 63}, 2)}) {
		if (v) {
			std::printf(" %016llx%016llx", ull(v->hi), ull(v->lo));
		} else {
			std::printf(" none");
		}
	}
	std::printf("\n");
	{
		auto hex = [](const std::array<std::uint8_t, 4> &sum) {
			std::printf(" %02x%02x%02x%02x", sum[0], sum[1], sum[2], sum[3]);
		};
		auto digest = api::Digest::new_();
		digest.sum_set(api::fingerprint("deadbeef").value());
		std::printf("digest");
		hex(digest.sum_get());
		hex(digest.flipped({0xff, 0, 0xff, 0}));
		print(" ", api::fingerprint("zz").error(), "\n");
	}
	{
		auto batch = api::Batch::new_();
		unsigned accepted = 0;
		for (std::uint32_t v = 1; v <= 5; v++) {
			accepted += batch.push(v);
		}
		auto batch2 = api::pair();
		unsigned paired = 0;
		for (std::uint32_t v = 1; v <= 3; v++) {
			paired += batch2.push(v);
		}
		auto emptied = api::drain_4(batch);
		std::printf("batch %zu %u %llu %llu %zu %u %llu\n", batch.capacity(), accepted, ull(emptied), ull(api::drain_4(batch)),
			batch2.capacity(), paired, ull(api::drain_2(batch2)));
		std::printf("capacities %zu %zu\n", api::capacities_4_2(batch, batch2), api::capacities_2_2(batch2, batch2));
	}
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
	api::Window window{{2, 8}, api::end(5)};
	std::printf("window %s %s ", yes(window.contains(4)), yes(window.contains(6)));
	print("", window.debug(), "\n");

	print("", api::greet("x", api::Shape::square), "\n");
	std::printf("len %zu\n", api::len("abc"));
	const std::uint8_t binary[] = {0xff, 0x00, 'a'};
	print("hex ", api::hex(api::view(binary, sizeof binary)), "\n");
	print("timed hex ", api::timed_hex(api::view(binary, sizeof binary)), "\n");
	print("sample hex ", api::sample_hex(api::view(binary, sizeof binary)), "\n");
	print("queued hex ", api::queued_hex(api::view(binary, sizeof binary)), "\n");
	print("tick ", api::tick_name(api::Label{api::buf("tick"), std::nullopt, std::nullopt}), "\n");
	print("rooted ", api::rooted_name(api::Label{api::buf("rooted"), std::nullopt, std::nullopt}), "\n");
	print("stored ", api::stored_text("stored"), "\n");
	print("layer ", api::layer_name(api::Label{api::buf("layer"), std::nullopt, std::nullopt}), "\n");
	api::Event events[] = {api::Event_Moved{p}, api::Event_Named{api::buf("n")}, api::Event_Done{}};
	for (const auto &event : events) {
		print("", api::describe(event), "\n");
	}

	// A `Copy` value passed by value is copied, so it stays usable.
	{
		api::Stamp stamp = api::Stamp::from_millis(2500);
		std::printf("secs %llu\n", ull(stamp.secs()));
		std::printf("later %s\n", yes(api::Stamp::from_millis(3000).later(stamp)));
		std::printf("secs %llu\n", ull(stamp.secs()));
		print("whole ", stamp.whole().debug(), "\n");
		api::Message message = api::Message::new_(stamp, "body");
		print("payload ", message.payload_get(), "\n");
		print("stamp ", message.stamp_get()->debug(), "\n");
		std::printf("sent %s\n", yes(message.sent()));
		// Setters assign in place: the bytes are copied in, and a `Copy` value copied.
		api::Stamp reset = api::Stamp::from_millis(4000);
		message.stamp_set(reset);
		message.payload_set("edited");
		print("payload ", message.payload_get(), "\n");
		print("stamp ", message.stamp_get()->debug(), "\n");
		std::printf("secs %llu\n", ull(reset.secs()));
		message.stamp_set(std::nullopt);
		std::printf("stamp %s\n", yes(message.stamp_get().has_value()));
	}

	// A value past a word, copied across: a closure takes and returns one by value.
	{
		api::Moment moment = *api::Moment::new_(1500, 1000);
		std::printf("moment %llu %s\n", ull(moment.millis()), yes(!api::Moment::new_(1, 0).has_value()));
		api::Moment shifted = api::shifted(moment, [](const api::Moment &m) { return m.after(500); });
		std::printf("shifted %llu %s\n", ull(shifted.millis()), yes(shifted == moment.after(500)));
		std::printf("wrapped %llu %s\n", ull(moment.after(UINT64_MAX).millis()), yes(moment == shifted));
	}

	// A padded value crosses with its padding zeroed, and one whose bytes vary is a handle.
	{
		api::Gap gap = api::Gap::new_(7, 3);
		std::printf("gap %llu %s\n", ull(gap.sum()), yes(gap == api::Gap::new_(7, 3)));
		api::Berth berth = api::Berth::new_(4u);
		std::printf("berth Some(%u) %s\n", *berth.at(), api::Berth::new_(std::nullopt).at() ? "Some" : "None");
	}

	// A `Copy` type changed in place is a value, whose change reaches only the copy changed.
	{
		api::Score score = api::Score::of(2);
		api::Score step = api::Score::of(3);
		score.add(step);
		score.add(step);
		std::printf("score %llu %llu %llu\n", ull(score.count()), ull(step.doubled().count()), ull(step.count()));
		api::Score copy = score;
		std::uint64_t taken = copy.take();
		std::printf("taken %llu %llu %llu\n", ull(taken), ull(copy.count()), ull(score.count()));
		api::Score other = step;
		auto [before0, before1] = copy.absorb(other);
		std::printf("absorbed %llu %llu %llu %llu %llu\n", ull(before0), ull(before1), ull(copy.count()),
			ull(other.count()), ull(step.count()));
		api::Hits hits = api::Hits::of(1, 2);
		api::Hits kept = hits;
		hits.add(api::Hits::of(10, 20));
		hits.hits_set(7);
		std::printf("hits %llu %u %llu %u %s\n", ull(hits.hits_get()), hits.misses_get(), ull(kept.hits_get()),
			kept.misses_get(), yes(hits == kept));
		api::Hits counted = kept;
		bool counted_same = counted.eq_(kept);
		std::printf("counted %s %llu\n", yes(counted_same), ull(counted.hits_get()));
		api::Extent extent = api::Extent::new_(5, 3);
		api::Extent extent_start = extent;
		std::uint64_t old = extent.slide(10);
		extent.start_set(extent.start_get() + 1);
		std::printf("extent %llu %llu %llu %llu %llu\n", ull(old), ull(extent.start_get()), ull(extent.end()),
			ull(extent_start.start_get()), ull(extent_start.end()));
		api::Weighted weighted{step, 4};
		std::printf("weighted %llu %llu\n", ull(weighted.total()), ull(weighted.score.count() + step.count()));
	}

	std::uint8_t levels[] = {1, 2, 3, 4};
	std::printf("kept %u\n", api::emit(std::make_unique<print_log>(), api::view(levels, sizeof levels)));
	{
		api::Logger logger = api::Logger::new_(std::make_unique<print_log>());
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
		api::Channel channel = api::Channel::new_();
		api::waker waker([]() { wakes++; });
		{
			api::Channel_recv_future recv = channel.recv();
			std::printf("recv pending %s\n", yes(!recv.poll(waker).has_value()));
			channel.send("hi");
			std::printf("woken %d\n", wakes);
			if (auto payload = recv.poll(waker)) {
				print("recv ", *payload, "\n");
			}
		}
		channel.send("");
		{
			api::Channel_recv_nonempty_future nonempty = channel.recv_nonempty();
			if (auto r = nonempty.poll(waker)) {
				if (*r) {
					std::printf("nonempty %zu\n", r->value().size());
				} else {
					print("nonempty err ", api::Error_to_string(r->error()), "\n");
				}
			} else {
				std::printf("nonempty pending\n");
			}
		}
		{
			auto fill4 = api::Batch::new_();
			auto fill2 = api::pair();
			auto filled4 = api::fill_4(fill4, 9).poll(waker);
			auto filled2 = api::fill_2(fill2, 1).poll(waker);
			if (filled4 && filled2) {
				std::printf("fill");
				for (auto v : *filled4) {
					std::printf(" %u", unsigned(v));
				}
				for (auto v : *filled2) {
					std::printf(" %u", unsigned(v));
				}
				std::printf("\n");
			}
		}

		// The send happens at the call: the future is dropped unpolled.
		channel.send_later("eager");
		{
			api::Channel_recv_future recv = channel.recv();
			if (auto payload = recv.poll(waker)) {
				print("recv ", *payload, "\n");
			}
		}
		// The send happens at the call, and the future borrows the channel and the tag.
		{
			api::Channel_tagged_future tagged = channel.tagged("tag");
			if (auto text = tagged.poll(waker)) {
				print("tagged ", *text, "\n");
			}
		}
		// So does a future borrowing an optional slice, which the export owns while it is pending.
		{
			api::Channel_itemized_future itemized = channel.itemized(std::vector<std::uint32_t>{1, 2, 3});
			if (auto count = itemized.poll(waker)) {
				std::printf("itemized %zu\n", *count);
			}
		}
		// Another crate's future borrowing its bytes, which the export owns while it is pending.
		{
			api::byte_len_future measuring = api::byte_len("abc");
			if (auto len = measuring.poll(waker)) {
				std::printf("byte_len %zu\n", *len);
			}
		}
		channel.send("sized");
		{
			api::Channel_recv_sized_future sized = channel.recv_sized();
			if (auto r = sized.poll(waker)) {
				print("sized ", std::get<0>(*r), " ");
				std::printf("%zu\n", std::get<1>(*r));
			}
		}
		// The waker moves into Rust, which keeps it until the send wakes it.
		channel.notify(waker);
		channel.send("rung");
		std::printf("woken %d\n", wakes);
		// A poll with no async twin is a future.
		{
			api::Channel_poll_take_future take = channel.poll_take();
			if (auto payload = take.poll(waker)) {
				print("take ", *payload, "\n");
			}
		}
		channel.send("measured");
		{
			api::Channel_poll_measure_future measure = channel.poll_measure(2);
			if (auto r = measure.poll(waker)) {
				std::printf("measure %zu\n", r->value());
			}
		}
		// A context is made of the waker, which Rust keeps until the send wakes it.
		channel.listen(waker);
		channel.send("heard");
		std::printf("woken %d\n", wakes);
		// A returned waker wakes the waiter's task, through Rust, until its last copy drops.
		api::Waiter waiter = api::Waiter::new_(waker);
		waiter.waker().wake();
		waiter.context().wake();
		std::printf("woken %d\n", wakes);
	}

	// A value that holds itself is an object.
	{
		api::Node list = api::Node::new_(1, std::nullopt);
		list.push(2);
		std::printf("node %llu\n", ull(list.sum()));
	}

	{
		api::Tally tally = api::Tally::new_();
		tally.bump();
		std::uint32_t count = tally.bump();
		std::printf("tally %u %s\n", count, yes(tally == tally));
		std::printf("both %u\n", api::bump_both(tally, tally));
		std::printf("peek %u\n", api::peek(tally, [](const api::Tally &lent) { return lent.bump(); }));
	}

	// Another export's types, which this export's function takes and returns.
	{
		api::Area area = api::grow(api::Area::new_({2, 3}), 1);
		api::Size size = area.size();
		std::printf("area %u %u %llu\n", size.width, size.height, ull(area.pixels()));
	}

	// Generic types, each at the instances it infers, named by their arguments.
	api::Pair_u32 pair{2, 5};
	api::Pair_u32 swapped = pair.swap();
	std::printf("pair %u %u sum %u ", swapped.left, swapped.right, pair.sum());
	print("", pair.debug(), " ");
	std::printf("%s\n", yes(pair == swapped));
	std::printf("even %s %s\n", yes(pair.even()), yes(api::Pair_u32{7, 7}.even()));
	// An instance renamed alone, beside another of `Pair` that keeps its name.
	api::Pair_Size_u8 sizes = api::tag({1, 2}, 3);
	api::Pair_u8_Size flipped = sizes.swap();
	std::printf("sized %u %u ", flipped.left, flipped.right.width);
	print("", flipped.debug(), "\n");
	api::Pair_u8_Size beside = api::Pair_u32{258, 9}.beside_Size(sizes.left);
	std::printf("beside %u ", beside.left);
	print("", beside.debug(), "\n");
	{
		// At its default, named as the type itself.
		api::Stack stack = api::Stack::new_();
		const char *names[] = {"a", "b"};
		for (int i = 0; i < 2; i++) {
			stack.push(api::Counter::new_(names[i], static_cast<std::uint64_t>(i) + 1));
		}
		std::optional<api::Counter> top = stack.pop();
		std::printf("stack %zu Some(%llu)", stack.depth(), ull(top->get()));
		std::printf(" %s", yes(stack.pop().has_value()));
		std::printf(" %s\n", yes(!stack.pop().has_value()));
		// Generic functions and methods, at each implementation of the trait bounding them,
		// by reference, which leaves C++ its handle. Two arguments sharing a last segment
		// name their instances apart by the module of one.
		api::Counter from_point = api::Counter::seeded_Point("p", {-3, 4});
		api::Counter seeded_Size = api::Counter::seeded_Size("s", {5, 6});
		api::Counter from_counter = api::Counter::seeded_Counter("c", from_point);
		api::Counter from_polar = api::Counter::seeded_polar_Point("q", {1, 2});
		std::printf("seeded %llu %llu %llu %llu past %llu seeds %llu\n", ull(from_point.get()), ull(seeded_Size.get()),
		            ull(from_counter.get()), ull(from_polar.get()), ull(stack.depth_past_Point({1, 2})),
		            ull(api::seeds_Point_Size({2, 3})));
	}

	// A trait implemented for `Point` and `&Point` apart: one instance.
	std::printf("weigh %u\n", api::weigh_Point({-1, 2}));
	// A function named as the type it returns, and one named as a type it does not.
	std::printf("polar %u %s area %llu\n", api::polar_point(2).value().distance, yes(!api::polar_point(0)),
	            ull(api::area({2, 3})));
	std::printf("wind %u\n", api::wind(api::PolarTurn{2}, api::polar_Turn{3}));
	// Names that clash in some language, each kept apart there.
	print("clash ", api::clash_Tone{440}.debug(), " ");
	print("", api::clash_TONE{3}.debug(), " ");
	print("", api::Cancelled{2}.debug(), "");
	api::clash_Span clash_span{4};
	std::printf(" span %u ", clash_span.width_());
	print("", clash_span.to_string_(), " ");
	print("", clash_span.to_string(), "");
	api::clash_Store clash_store = api::clash_Store::new_(7);
	std::printf(" store %u %u", clash_store.free_(), clash_store.close());
	std::printf(" audio %u %u nested %u\n", api::clash_audio_encoder_finish(), api::clash_audio_encoder_finish_(),
	            api::clash_nested(std::make_tuple(std::make_tuple(std::uint8_t{1}, std::uint16_t{2}), 3u),
	                              std::make_tuple(std::make_tuple(std::uint8_t{1}, std::uint16_t{2}, 3u))));
	// Instances that name each other, and one made only by an impl of it.
	std::printf("cross %u outer %u\n", api::cross(api::Left_Right_u8{api::Right_u8{1}}, api::Right_Left_u8{api::Left_u8{2}}),
	            api::Outer_Inner_u8::new_(4).get());
	// A bound that outlives the call takes a value, and an instance only a constant names.
	std::printf("kept %llu boxed %u\n", ull(api::kept_Point({1, -2})), api::BOXED().value);
	std::printf("keep %d\n", api::keep_Point({1, -2}));
	std::printf("kept or %d %d keeper %d\n", api::kept_or_Point(api::Point{5, 2}), api::kept_or_Point(std::nullopt),
		api::keeper_Point({5, 2}));
	// Inferred instances taking a lent closure and a foreign implementation, and one only
	// its impl of a trait names.
	api::Pair_u32 sides{2, 5};
	std::printf("apply %u\n", api::apply_u32(sides, [](std::uint32_t side) { return side * 3; }));
	std::printf("report %u\n", api::report_u32(sides, std::make_unique<print_log>()));
	std::printf("wrapped %llu\n", ull(api::kept_Wrap_u32({4})));
	std::printf("keeps %u\n", api::keeps(std::make_unique<print_log>()).value);

	// Its Drop runs when C++ destroys the handle, not before.
	{
		api::Token token = api::Token::new_(7);
		std::printf("token %u dropped %u\n", token.id_get(), api::dropped());
	}
	std::printf("dropped %u\n", api::dropped());

	// A field and the method of its name, each its own member.
	api::Retry retry = api::Retry::new_(5u);
	std::optional<std::uint32_t> limit = retry.limit_get();
	std::printf("retry %d %u %u %u\n", limit.has_value(), limit.value_or(0), retry.limit(), retry.delay());
	// The renamed field's setter, named as its getter.
	retry.limit_set(std::nullopt);
	std::printf("retry %u\n", retry.limit());
	std::printf("slot %u\n", api::Slot_u8::new_(200).value());

	// Foreign types: a URL parsed from text, socket addresses, OS paths, and `Vec`s.
	api::Url base = api::Url::parse("https://example.com/a/b").value();
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
	if (auto parsed = api::Url::parse("no scheme")) {
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
	api::Peer peer = api::Peer{v6, names}.next("c");
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
	std::printf("either %zu %zu\n", api::either("abc"), api::either(std::nullopt));
	print("prefixed ", api::prefixed("cd", api::view("ab")), " ");
	print("", api::prefixed("cd", std::nullopt), "\n");
	std::printf("coordinates %d %d\n", api::coordinates(api::Point{2, 3}), api::coordinates(std::nullopt));
	{
		auto two = api::Counter::new_("two", 2);
		auto three = api::Counter::new_("three", 3);
		std::printf("plus %llu %llu\n", static_cast<unsigned long long>(two.plus(&three)),
			static_cast<unsigned long long>(two.plus(nullptr)));
		bool added = two.add_to(&three);
		bool added_none = two.add_to(nullptr);
		std::printf("add_to %s %s %llu\n", added ? "true" : "false", added_none ? "true" : "false",
			static_cast<unsigned long long>(three.get()));
	}

	{
		// A value passed for &mut, which takes what Rust wrote back.
		api::buf encoded(api::view("0,"));
		api::put_number(1, encoded);
		api::put_number(23, encoded);
		print("put_number ", encoded, "\n");
		::std::tuple<api::buf, uint32_t> entry(api::buf(api::view("ab")), 1);
		uint32_t stamps = api::push_counted(entry, 'c');
		std::printf("push_counted %u ", stamps);
		print("", ::std::get<0>(entry), " ");
		std::printf("%u\n", ::std::get<1>(entry));
		api::buf digits(api::view("7x9"));
		auto first_digit = api::digit(digits);
		auto skipped = api::digit(digits);
		auto last_digit = api::digit(digits);
		std::printf("digit %u %s %u %zu\n", unsigned(first_digit.value()), yes(!skipped.has_value()),
			unsigned(last_digit.value()), digits.size());
		api::Point nudged{1, 2};
		api::nudge(nudged, 3);
		std::printf("nudge %d %d\n", nudged.x, nudged.y);
	}
	{
		auto config = api::secure("h3");
		print("secure ", api::alpn_lent(config), " ");
		print("", api::alpn(config), "\n");
		auto cert = api::certify("der");
		std::printf("cert %llu ", ull(api::cert_len(cert)));
		print("", api::cert_der(std::move(cert)), " ");
		std::vector<api::tls_Cert> chain;
		chain.push_back(api::certify("a"));
		chain.push_back(api::certify("bc"));
		std::printf("%llu\n", ull(api::chain_len(std::move(chain))));
		auto session = api::resume("h3", 60);
		print("resume ", session.value().alpn(), " ");
		std::printf("%u\n", unsigned(session.value().lifetime().seconds));
		print("resume err ", api::resume("", 60).error(), "\n");
		auto meters = api::Meters::new_();
		auto first = meters.first();
		first.add(2);
		unsigned long long added = first.add(3);
		std::printf("lent %llu %llu\n", added, ull(meters.first().count()));
		meters.clear();
		std::printf("cleared %llu\n", ull(meters.first().count()));
		auto last = meters.last_get();
		last.add(4);
		std::printf("field lent %llu\n", ull(last.add(1)));
		auto tallies = api::Tallies::new_();
		tallies.first().bump();
		std::printf("tallied %u\n", tallies.first().bump());
	}

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
	std::vector<api::Counter> some, boxed;
	some.push_back(std::move(counters[0]));
	boxed.push_back(std::move(counters[1]));
	auto worth = api::worth(std::move(some), std::move(boxed));
	std::printf("worth %llu %llu\n", ull(worth), ull(api::worth(std::nullopt, {})));
	std::optional<std::uint64_t> counts[] = {
		api::held_count(std::tuple{api::Counter::new_("h", 4)}),
		api::held_count(std::nullopt),
	};
	std::printf("held");
	for (auto count : counts) {
		if (count) {
			std::printf(" Some(%llu)", ull(*count));
		} else {
			std::printf(" None");
		}
	}
	std::printf("\n");
	print("boxed Some(\"", *api::boxed_name(api::boxed_counter("b")), "\") ");
	std::printf("%s %s\n", api::boxed_name(std::nullopt) ? "Some" : "None", api::boxed_counter(std::nullopt) ? "false" : "true");
	std::printf("less %u\n", api::less(5, 2));

	auto sums = api::sums({{1, 2}, {}, {3}});
	std::printf("sums [%u, %u, %u]\n", sums[0], sums[1], sums[2]);
	{
		// Fields named as an `operator==`'s argument might be.
		api::Shadowed one{{1}, 2, 3};
		api::Shadowed two{{1}, 2, 4};
		if (!(one == one) || one == two || !(one != two)) {
			std::fprintf(stderr, "a record with fields named other, a, and b compares wrong\n");
			std::abort();
		}
	}
	{
		// Types named as an `operator==`'s arguments might be.
		api::a one{1};
		api::a two{2};
		api::b unit = api::b_One{};
		api::b two_b = api::b_Two{2};
		auto handle = api::c::new_(1);
		if (!(one == one) || one == two || !(unit != two_b) || !(handle == api::c::new_(1)) || handle == api::c::new_(2)) {
			std::fprintf(stderr, "types named a, b, and c compare wrong\n");
			std::abort();
		}
	}
	std::printf("least");
	for (const auto &set : api::least({{3, 1}, {}, {2}})) {
		std::printf(" [");
		for (std::size_t j = 0; j < set.size(); j++) {
			std::printf(j ? ", %u" : "%u", set[j]);
		}
		std::printf("]");
	}
	std::printf("\n");
	numbers("edges", api::Numbers::edges());
	numbers("reversed", api::Numbers::edges().reversed());
	{
		auto edges = api::Numbers::edges();
		auto nan = api::Measure::new_(std::nan(""), false);
		auto same = [](bool b) { return b ? "true" : "false"; };
		std::printf("float equal %s %s %s %s %s\n", same(edges == edges), same(edges == api::Numbers::edges()),
		            same(api::Numbers::zero(true) == api::Numbers::zero(false)), same(nan == nan),
		            same(api::Measure::new_(-0.0, true) == api::Measure::new_(0.0, false)));
	}
	std::printf("sum u32 %llu %llu\n", static_cast<unsigned long long>(api::sum_u32({})),
	            static_cast<unsigned long long>(api::sum_u32({UINT32_MAX, UINT32_MAX, 2})));
	std::printf("doubled i64 ");
	print_list(api::doubled_i64({INT64_MIN, -1, 0, INT64_MAX}), [](int64_t v) { return std::to_string(v); });
	std::printf("\n");
	std::printf("boxed floats ");
	print_list(api::boxed_floats({-0.0, f64_from(0x7ff8000000000001)}), [](double v) { return std::to_string(f64_bits(v)); });
	std::printf("\n");
	auto summed = api::summed({{1, 2}, {-4, 6}});
	auto none_summed = api::summed({});
	std::printf("summed %d %d %d %d\n", summed.x, summed.y, none_summed.x, none_summed.y);
	std::printf("longest %zu %zu\n", api::longest({"a", "abc"}), api::longest({}));
	print("first of Some(\"", api::first_of({"a", "b"}).value(), "\") ");
	std::printf("%s\n", api::first_of({}) ? "Some" : "None");
	auto limits = api::byte_limits("a", "bc");
	print("limits ", limits.low, " ");
	print("", limits.high, " ");
	auto breadth = api::breadth({{0, 5000000}, {2, 0}});
	std::printf("breadth %llu\n", static_cast<unsigned long long>(breadth.secs * 1000 + breadth.nanos / 1000000));
	print("joined ", api::joined({"ab", "", "c"}), " ");
	std::printf("%zu read %llu\n", api::joined({}).size(), static_cast<unsigned long long>(api::read_len("hello")));

	api::Json doc = api::Json::parse(R"({"b": [1, 2.5, null], "a": "\u00e9"})").value();
	print("json ", api::encode(doc), "\n");
	print("timed text ", api::timed_text(doc), "\n");
	api::Json tagged = api::tagged(std::move(doc), "t");
	print("tagged ", tagged.to_text(), "\n");
	{
		// A JSON object's entries cross as pairs of a key and its document, each moved in.
		std::vector<std::tuple<api::view, api::Json>> pairs;
		pairs.emplace_back("b", api::Json::parse("[1]").value());
		pairs.emplace_back("a", api::Json::parse("2").value());
		auto entries = api::entries(std::move(pairs), "t");
		std::printf("entries");
		for (std::size_t i = 0; i < entries.size(); i++) {
			auto &[key, value] = entries[i];
			print(i == 0 ? " " : ", ", key, " ");
			print("", value.to_text(), "");
		}
		std::printf("\n");
	}
	if (auto parsed = api::Json::parse("{\"a\":")) {
		print("json parse ", parsed->to_text(), "\n");
	} else {
		print("json parse err ", parsed.error(), "\n");
	}

	api::Site site = api::Site::new_(base);
	print("site ", site.home_get().to_string(), " ");
	print("", site.home_get().to_string(), "\n");

	// Slices, passed in and returned as clones, and the socket address of each family.
	auto shapes = api::Shape_ALL();
	std::printf("shapes [");
	for (std::size_t i = 0; i < shapes.size(); i++) {
		print(i == 0 ? "" : ", ", api::Shape_debug(shapes[i]), "");
	}
	std::printf("] greetings ");
	print_strings(api::GREETINGS());
	std::printf("\n");
	api::Trail trail = api::Trail::new_({api::Point{1, 2}, api::Point{3, 4}}, {"a", "b"});
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
	// A filter, collected at the call.
	auto beyond = trail.beyond(1);
	std::printf("beyond");
	for (std::size_t i = 0; i < beyond.size(); i++) {
		std::printf("%s%d %d", i == 0 ? " " : ", ", beyond[i].x, beyond[i].y);
	}
	std::printf("\n");
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

	// serde_json's map iterator, collected as pairs of a key and its document.
	{
		std::vector<std::tuple<api::view, api::Json>> pairs;
		pairs.emplace_back("b", api::Json::parse("[1]").value());
		pairs.emplace_back("a", api::Json::parse("2").value());
		auto sections = api::sections(std::move(pairs));
		std::printf("sections");
		for (std::size_t i = 0; i < sections.size(); i++) {
			auto &[key, value] = sections[i];
			print(i == 0 ? " " : ", ", key, " ");
			print("", value.to_text(), "");
		}
		std::printf("\n");
	}
	// An instant only Rust makes, which each call copies out of its handle, and compares.
	{
		auto millis = [](api::duration elapsed) { return static_cast<unsigned long long>(elapsed.secs * 1000 + elapsed.nanos / 1000000); };
		auto began = api::now();
		auto ended = api::later(began, {1, 500000000});
		std::printf("instants %llu %llu %s %s\n", millis(api::since(ended, began)), millis(api::since(began, ended)),
					began == api::later(began, {0, 0}) ? "true" : "false", began == ended ? "true" : "false");
		// An instant lent inside an `Option`, which C++ copies.
		std::printf("has_instant %s %s\n", api::has_instant(began) ? "true" : "false", api::has_instant(std::nullopt) ? "true" : "false");
	}
	// tracing's level, and its filter as an optional level whose empty is `OFF`.
	{
		auto verbosity = api::Verbosity::new_(api::Level::warn);
		print("verbosity ", api::spell(verbosity.filter()), " ");
		print("", api::spell(api::louder(verbosity.filter())), "\n");
		verbosity.level_set(api::Level::trace);
		std::printf("verbosity %s ", verbosity.level_get() == api::Level::trace ? "true" : "false");
		print("", api::spell(verbosity.filter()), " ");
		print("", api::spell(api::louder(verbosity.filter())), " ");
		print("", api::spell(api::louder(std::nullopt)), "\n");
	}

	// Closures C++ implements that return what Rust then owns, or that Rust keeps.
	print("made ", api::made(3, [](std::uint32_t n) { return std::to_string(n) + " times"; }), "\n");
	// A closure taking and returning `Copy` values copies them, never running their `Clone`.
	{
		api::Ticket kept = api::Ticket::new_(9);
		std::uint32_t given =
			api::reissue(api::Ticket::new_(7), [](const api::Ticket &t) { return api::Ticket::new_(t.number() + 1); })
				.number();
		std::uint32_t held = api::reissue(api::Ticket::new_(7), [&kept](const api::Ticket &) { return kept; }).number();
		std::printf("reissued %u %u, cloned %u\n", static_cast<unsigned>(given), static_cast<unsigned>(held),
			static_cast<unsigned>(api::Ticket::clones()));
	}
	{
		// `Copy` objects with no C++ copy, beside bytes: a returned vector moves back in,
		// stored or not, each ticket moved and each name viewed in place.
		std::vector<std::tuple<api::view, api::Ticket>> lent;
		lent.emplace_back("a", api::Ticket::new_(1));
		auto returned = api::tickets(lent);
		auto again = api::tickets(api::tickets(std::move(returned)));
		if (again.size() != 1 || std::get<0>(again[0]).str() != "a" || std::get<1>(again[0]).number() != 1) {
			std::fprintf(stderr, "returned tickets passed back wrong\n");
			std::abort();
		}
	}
#ifdef __cpp_lib_ranges
	{
		// A range whose end is a sentinel of another type than its begin.
		std::vector<std::string> letters{"p", "q", "r"};
		auto taken = api::counters(letters | std::views::take_while([](const std::string &n) { return n != "r"; }));
		if (taken.size() != 2 || taken[1].name().str() != "q") {
			std::fprintf(stderr, "a sentinel range passed wrong\n");
			std::abort();
		}
	}
#endif
	// A kept closure, called on another thread after the call that took it returned.
	std::printf("spawned %u\n", static_cast<unsigned>(api::Spawned::new_([] { return 7; }).join()));
	// A kept closure: called again at the instant it asks for, then once its waiter wakes.
	{
		auto began = api::now();
		int calls = 0;
		auto drive = api::drive(began, [&](const api::Instant &now, const api::Waiter &waiter) {
			using result = api::result<std::optional<api::Instant>, std::string>;
			calls++;
			if (calls == 1) {
				return result::ok(api::later(now, {0, 5000000}));
			}
			if (calls == 2) {
				api::duration since = api::since(now, began);
				std::printf("drive %llu\n", static_cast<unsigned long long>(since.secs * 1000 + since.nanos / 1000000));
				waiter.waker().wake();
				return result::ok(std::nullopt);
			}
			return result::err("stopped after " + std::to_string(calls));
		});
		api::waker waker([] {});
		std::optional<api::buf> stopped;
		while (!(stopped = drive.poll(waker))) {
		}
		print("drive ", *stopped, "\n");
	}
	{
		memory hello{"hello world"};
		tail(hello);
		broken_reader broken;
		tail(broken);
		// A seek from the start reaches the reader whole, past `INT64_MAX`, as `Cursor` takes one.
		far_reader far;
		auto sought = api::seek_to(far, UINT64_MAX);
		if (sought) {
			std::printf("sought %llu\n", static_cast<unsigned long long>(*sought));
		} else {
			print("sought error ", sought.error(), "\n");
		}
	}
	print("non zeros ", api::describe_non_zeros(api::non_zeros()), "\n");
	for (auto hop : {std::optional<std::uint64_t>{}, std::optional<std::uint64_t>{1}, std::optional<std::uint64_t>{UINT64_MAX}}) {
		if (auto next = api::next_hop(hop)) {
			std::printf("hop Some(%llu)\n", static_cast<unsigned long long>(*next));
		} else {
			std::printf("hop None\n");
		}
	}
	std::printf("farther hop %u\n", static_cast<unsigned>(api::farther_hop(api::Pair_non_zero_u16{3, UINT16_MAX})));

	// A returned `Vec<&Token>` holds clones, which the vector drops.
	api::Token twice = api::Token::new_(9);
	std::uint32_t before = api::dropped();
	{
		auto tokens = twice.twice();
		if (tokens.size() != 2 || tokens[1].id_get() != 9) {
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

// A future C++ implements, which Rust polls: ready on its `ready`th poll, keeping the
// waker of the last in what it shares with the test.
struct countdown {
	struct state {
		int ready;
		int polls = 0;
		std::optional<api::waker> waker;
		explicit state(int r) : ready(r) {}
	};
	std::shared_ptr<state> s;

	bool poll(const api::waker &w) {
		if (++s->polls >= s->ready) {
			s->waker.reset();
			return true;
		}
		s->waker = w;
		return false;
	}
};

int foreign_futures() {
	// Ready at once.
	{
		auto s = std::make_shared<countdown::state>(1);
		check(api::after(countdown{s}, 4).get() == 8 && s->polls == 1, "a future C++ implements is awaited");
		check(s.use_count() == 1, "Rust destroys a future C++ implements once done with it");
	}
	// Pending once: its wake reaches the waker Rust was polled with.
	{
		auto s = std::make_shared<countdown::state>(2);
		api::after_future after = api::after(countdown{s}, 5);
		int woken = 0;
		api::waker waker([&]() { woken++; });
		check(!after.poll(waker) && s->waker, "a future C++ implements pends");
		s->waker->wake();
		check(woken == 1, "its wake reaches Rust's poller");
		auto got = after.poll(waker);
		check(got && *got == 10 && s.use_count() == 1, "it finishes once polled again, and is destroyed");
	}
	// Never ready: Rust gives up on it, destroying it with the waker it kept.
	{
		auto s = std::make_shared<countdown::state>(1000);
		auto got = api::within(0, countdown{s}).get();
		check(!got && std::holds_alternative<api::Error_Overflow>(got.error()), "Rust gives up on a future C++ implements");
		check(s.use_count() == 1 && s->polls == 1, "and destroys it");
	}
	// A binding's future of void is one too.
	{
		api::Channel channel = api::Channel::new_();
		check(api::after(channel.send_later("passed"), 6).get() == 12, "a binding's future of void passes to Rust");
		check(channel.recv().get().str() == "passed", "the future Rust polled ran");
	}
	// Ready in time, by a second poll.
	{
		auto s = std::make_shared<countdown::state>(2);
		api::within_future within = api::within(1, countdown{s});
		api::waker waker([]() {});
		check(!within.poll(waker), "a future C++ implements pends once");
		check(within.poll(waker) && s->polls == 2, "a future C++ implements finishes in time");
	}
	// A poll of a waiter, made of each poll's waker, of a future C++ implements, which the
	// future owns and lends it pinned: destroyed once it finishes.
	{
		auto s = std::make_shared<countdown::state>(2);
		api::Waiter_poll_future_future polled = api::Waiter::poll_future(countdown{s});
		int woken = 0;
		api::waker waker([&]() { woken++; });
		check(!polled.poll(waker) && s->waker, "a future a poll lends pends");
		s->waker->wake();
		check(woken == 1 && s.use_count() == 2, "its wake reaches Rust's poller, and the future keeps it");
		check(polled.poll(waker) && s->polls == 2 && s.use_count() == 1, "it finishes once polled again, and is destroyed");
	}
	// Dropped before it finishes, it destroys the future it owns.
	{
		auto s = std::make_shared<countdown::state>(1000);
		{
			api::Waiter_poll_future_future polled = api::Waiter::poll_future(countdown{s});
			api::waker waker([]() {});
			check(!polled.poll(waker), "a future a poll lends pends");
		}
		check(s.use_count() == 1, "a dropped future destroys the future it owns");
	}
	// A kept closure, destroyed once Rust drops it: after its thread ran it.
	{
		auto kept = std::make_shared<int>(7);
		api::Spawned spawned = api::Spawned::new_([kept] { return static_cast<std::uint32_t>(*kept); });
		check(std::move(spawned).join() == 7 && kept.use_count() == 1, "Rust destroys a closure it kept once done with it");
	}
	// A kept closure the future of the call holds, destroyed with it before it finishes.
	{
		auto kept = std::make_shared<int>(0);
		auto start = api::now();
		{
			auto drive = api::drive(start, [kept](const api::Instant &, const api::Waiter &) {
				return api::result<std::optional<api::Instant>, api::buf>::ok(std::nullopt);
			});
			check(kept.use_count() == 2, "Rust keeps its own copy of a closure");
			api::waker waker([]() {});
			check(!drive.poll(waker) && kept.use_count() == 2, "a future keeps the closure while it pends");
		}
		check(kept.use_count() == 1, "a dropped future destroys the closure it kept");
	}
	// A closure failing with an error any view takes, and a reader's seek failing.
	{
		auto start = api::now();
		auto drive = api::drive(start, [](const api::Instant &, const api::Waiter &) {
			return api::result<std::optional<api::Instant>, const char *>::err("failed");
		});
		check(std::move(drive).get().str() == "failed", "a closure's error reaches Rust as its message");
		memory empty{""};
		empty.at = 0;
		auto read = api::tail(empty, 5);
		check(read && std::get<0>(*read).empty() && std::get<1>(*read) == 0, "a reader of nothing reads nothing");
	}
	return 0;
}

int futures() {
	foreign_futures();
	api::Channel channel = api::Channel::new_();

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
		auto read = [&](api::Channel &ch) -> detached {
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

// Who owns each handle and bytes, beyond the transcript, each checked by an assertion.
// A payload Rust returned goes back to Rust in place: moved in, shared by a copy kept
// meanwhile, and written back through `&mut`. Its bytes stay where Rust put them, and
// drop once, with their last owner.
void forwarding() {
	std::uint32_t before = api::dropped();
	api::Channel channel = api::Channel::new_();
	api::buf payload = api::token_bytes(3, 64);
	const std::uint8_t *at = payload.data();
	channel.send(std::move(payload));
	payload = channel.recv().get();
	check(payload.data() == at && payload.size() == 64, "a payload moved into Rust stays in place");
	api::buf kept = payload;
	check(kept.data() == at, "a copy of Rust's bytes shares them");
	channel.send(kept);
	check(channel.recv().get().data() != at, "bytes lent to a call are copied");
	channel.send(std::move(kept));
	api::buf shared = channel.recv().get();
	check(shared.data() == at && payload.data() == at, "a copy moved into Rust stays in place");
	api::Packet packet = api::next_packet(api::Packet{1, std::move(shared)});
	check(packet.payload.size() == 64 && api::dropped() == before, "a record carries the payload on");
	payload = api::buf();
	packet = api::Packet{};
	check(api::dropped() == before + 1, "the payload drops once, with its last owner");
	api::buf digits = api::token_bytes('0', 2);
	api::put_number(7, digits);
	check(digits.str() == "007," && api::dropped() == before + 2, "Rust takes its bytes passed for &mut");
}

int lifetimes() {
	// Copies of a type whose clone no call tells apart share one handle: a write through
	// one shows in the other, as it does between clones of an `Arc`.
	{
		std::uint32_t before = api::dropped();
		api::Gauge a = api::Gauge::new_(1);
		api::Gauge b = a;
		b.set(2);
		check(a.level() == 2, "copies of a shared type see each other's writes");
		// Moving out of one copy moves out a clone, as Rust moves out of a clone, and
		// leaves the other copy whole.
		check(std::move(b).into_level() == 2, "a shared copy moves out its value");
		check(a.level() == 2 && api::dropped() == before, "moving out of a shared copy leaves the others");
		{
			api::Gauge c = a;
			(void)c;
		}
		check(api::dropped() == before, "a released copy frees nothing while another holds it");
		// The last copy moves out the value itself.
		check(std::move(a).into_level() == 2 && api::dropped() == before + 1, "the last copy moves out the value");
	}

	// A pending future borrows the value as though it held a copy: once the copy that made
	// it goes, moving out of the last copy moves out a clone, as Rust moves out of a clone
	// while a future borrows another.
	{
		std::uint32_t before = api::dropped();
		api::Gauge a = api::Gauge::new_(4);
		// On the heap, where GCC 16 sees no use of an uninitialized `optional`.
		auto made = std::make_unique<api::Gauge>(a);
		api::Gauge_later_future later = made->later();
		api::waker waker([]() {});
		check(!later.poll(waker), "a future pends");
		made.reset();
		check(std::move(a).into_level() == 4 && api::dropped() == before, "the last copy moves out a clone while a future borrows it");
		check(std::move(later).get() == 4 && api::dropped() == before + 1, "a future reads the value it borrows, and frees it once it finishes");
	}

	// Copies of a type whose clone is a value of its own, such as a lock behind a `Box`,
	// keep apart: copying is `clone()`, which crosses into Rust, and a write through one
	// clone never shows in the other.
	{
		api::Latch a = api::Latch::default_();
		api::Latch b = a.clone();
		check(b.set(1) && !a.get(), "a clone's write stays its own");
		check(a.set(2) && a.get() == 2u && b.get() == 1u, "each clone holds its own value");
	}

	// A copy released on one thread while a call runs on another: the call's copy keeps
	// the handle, and the last copy frees it, once, on whichever thread drops it.
	{
		std::uint32_t before = api::dropped();
		std::optional<api::Gauge> mine(api::Gauge::new_(7));
		std::mutex mutex;
		std::condition_variable cv;
		bool started = false;
		bool released = false;
		bool read = true;
		std::thread caller([&, theirs = *mine]() {
			{
				std::lock_guard<std::mutex> lock(mutex);
				started = true;
			}
			cv.notify_all();
			for (;;) {
				bool last;
				{
					std::lock_guard<std::mutex> lock(mutex);
					last = released;
				}
				read = read && theirs.level() == 7;
				if (last) {
					return;
				}
			}
		});
		{
			std::unique_lock<std::mutex> lock(mutex);
			cv.wait(lock, [&]() { return started; });
		}
		mine.reset();
		{
			std::lock_guard<std::mutex> lock(mutex);
			released = true;
		}
		caller.join();
		check(read, "a call races a copy's release unharmed");
		check(api::dropped() == before + 1, "the last copy frees the value once");
	}

	// A copy of a lent object lasts only its call, as the lent object does.
	{
		api::Gauge gauge = api::Gauge::new_(3);
		check(api::watch(gauge, [](const api::Gauge &lent) {
			api::Gauge copy = lent;
			return copy.level();
		}) == 3, "a copy of a lent object reads it during the call");
	}

	// Returned bytes own Rust's buffer, so they outlive the object they came from; a view
	// or span of them lasts only as long as they do.
	{
		api::buf payload;
		{
			api::Message message = api::Message::new_(std::nullopt, "kept");
			payload = message.payload_get();
		}
		check(payload.str() == "kept", "bytes outlive the object they came from");
		std::uint32_t before = api::dropped();
		{
			api::buf bytes = api::token_bytes(9, 3);
			check(api::dropped() == before && bytes.size() == 3 && bytes.data()[2] == 9, "bytes hold their owner");
#ifdef __cpp_lib_span
			// From C++20, a view is a contiguous range: a span of it, and a view of a span.
			std::span<const std::uint8_t> span = api::view(bytes);
			check(span.size() == 3 && span[0] == 9, "a view converts to a span");
			check(api::len(span) == 3, "a span converts to a view");
#endif
		}
		check(api::dropped() == before + 1, "bytes free their owner");
	}

	forwarding();

	std::printf("lifetimes ok\n");
	return 0;
}

// Each breaks a contract, which must abort with a message naming the function. Those
// `tests/api_strict.cpp` refuses at compile time reach the object through a reference or
// pointer, whose state Clang does not track.
int broken(const char *which) {
#ifdef __linux__
	// Non-dumpable, the expected abort writes no core and never starts the host's crash
	// handler.
	if (prctl(PR_SET_DUMPABLE, 0, 0, 0, 0) != 0) {
		std::perror("prctl");
		return 1;
	}
#endif
	if (std::strcmp(which, "moved") == 0) {
		api::Counter c = api::Counter::new_("c", 0);
		api::Counter &same = c;
		api::Label label = std::move(c).into_label(std::nullopt);
		same.get();
	} else if (std::strcmp(which, "moved from") == 0) {
		api::Counter c = api::Counter::new_("c", 0);
		api::Counter &same = c;
		api::Counter d = std::move(c);
		same.get();
	} else if (std::strcmp(which, "shared moved from") == 0) {
		api::Gauge g = api::Gauge::new_(0);
		api::Gauge &same = g;
		api::Gauge h = std::move(g);
		std::move(same).into_level();
	} else if (std::strcmp(which, "shared moved out") == 0) {
		api::Gauge g = api::Gauge::new_(0);
		api::Gauge &same = g;
		std::move(g).into_level();
		std::move(same).into_level();
	} else if (std::strcmp(which, "lent copied") == 0) {
		static std::optional<api::Gauge> kept;
		api::watch(api::Gauge::new_(0), [](const api::Gauge &lent) {
			kept.emplace(lent);
			return lent.level();
		});
	} else if (std::strcmp(which, "value") == 0) {
		api::Counter c = api::Counter::new_("c", 0);
		c.sub(1).value();
	} else if (std::strcmp(which, "finished") == 0) {
		api::Channel channel = api::Channel::new_();
		channel.send("x");
		api::Channel_recv_future recv = channel.recv();
		api::waker waker([]() {});
		recv.poll(waker);
		recv.poll(waker);
	} else if (std::strcmp(which, "null view") == 0) {
		api::len(api::view(nullptr, 1));
	} else if (std::strcmp(which, "outlived") == 0) {
		api::Meters meters = api::Meters::new_();
		api::Meter first = meters.first();
		meters.clear();
		first.count();
	} else if (std::strcmp(which, "lent moved") == 0) {
		api::Registry registry = api::Registry::new_(api::Counter::new_("c", 0));
		registry.edit([](api::borrow<api::Counter> counter) {
			api::Label label = std::move(*counter.operator->()).into_label(std::nullopt);
			return false;
		});
	} else if (std::strcmp(which, "overread") == 0) {
		struct overread {
			api::result<std::size_t, std::string> read(std::uint8_t *, std::size_t len) {
				return api::result<std::size_t, std::string>::ok(len + 1);
			}
			api::result<std::uint64_t, std::string> seek(std::int64_t, std::ios_base::seekdir) {
				return api::result<std::uint64_t, std::string>::ok(0);
			}
		} file;
		api::tail(file, 1);
	} else if (std::strcmp(which, "lent kept") == 0) {
		static std::optional<api::Counter> kept;
		api::Registry registry = api::Registry::new_(api::Counter::new_("c", 0));
		registry.edit([](api::borrow<api::Counter> counter) {
			kept.emplace(std::move(*counter.operator->()));
			return false;
		});
	}
	return 0;
}

// A method of dep-fixture's `Area` at the api fixture's `Point`, which the crate header of
// `Point`'s crate defines, though `Area`'s class declares it.
int rehomed() {
	api::Area area = api::Area::new_({2, 3});
	std::printf("scaled by %llu\n", static_cast<unsigned long long>(area.scaled_Point({-3, 4}) / area.pixels()));
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
	if (argc > 1 && std::strcmp(argv[1], "lifetimes") == 0) {
		return lifetimes();
	}
	if (argc > 1 && std::strcmp(argv[1], "rehomed") == 0) {
		return rehomed();
	}
	if (argc > 1) {
		return broken(argv[1]);
	}
	return script();
}
