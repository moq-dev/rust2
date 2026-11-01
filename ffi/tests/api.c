/* The api fixture's C test: the script fixtures/api/examples/transcript.rs runs
 * natively, through the exported C ABI. tests/fixtures.rs compares the transcripts.
 *
 * With an argument, it instead breaks one contract of the ABI, which must abort. */

#include <stdio.h>
#include <string.h>

#include "api.h"

static api_buf str(const char *s) {
	return (api_buf){.ptr = (const uint8_t *)s, .len = strlen(s)};
}

/* Print a returned buffer, then free it. */
static void print(const char *before, api_buf buf, const char *after) {
	printf("%s%.*s%s", before, (int)buf.len, (const char *)buf.ptr, after);
	api_buf_free(buf);
}

static int wakes = 0;
static int drops = 0;

static void wake(void *data) {
	(void)data;
	wakes++;
}

static void drop(void *data) {
	(void)data;
	drops++;
}

/* The Log callback: keeps even levels. */
static bool log_line(void *data, uint8_t level, api_buf line) {
	(void)data;
	printf("log %u %.*s\n", level, (int)line.len, (const char *)line.ptr);
	return level % 2 == 0;
}

static void log_free(void *data) {
	(void)data;
	printf("freed\n");
}

/* A lent closure: adds data's amount to the counter lent for the call. */
static bool edit(void *data, api_counter *counter) {
	api_counter_add(counter, *(const uint64_t *)data, NULL, NULL);
	return api_counter_edited(counter) && api_counter_get(counter) == 6;
}

/* Lent closures borrowing their object shared: one `Sync`, one not. */
static uint32_t seven(void *data) {
	(void)data;
	return 7;
}

/* A lent closure of an inferred instance's type. */
static uint32_t triple(void *data, uint32_t side) {
	(void)data;
	return side * 3;
}

static uint64_t visit(void *data, const api_cursor *cursor) {
	(void)data;
	return api_cursor_at(cursor) * 2;
}

static void print_visit(void *data, uint32_t at) {
	(void)data;
	printf("visit %u\n", at);
}

static uint32_t peek_tally(void *data, const api_tally *tally) {
	(void)data;
	return api_tally_bump(tally);
}

/* A lent closure, which Rust calls on another thread: keeps levels above data's. */
static bool keep_line(void *data, uint8_t level, api_buf line) {
	printf("filter %u %.*s\n", level, (int)line.len, (const char *)line.ptr);
	return level > *(const uint8_t *)data;
}

/* Breaks a lent handle's contract: frees it, or moves its value out. */
static bool free_lent(void *data, api_counter *counter) {
	(void)data;
	api_counter_free(counter);
	return false;
}

static bool move_lent(void *data, api_counter *counter) {
	(void)data;
	api_label label = api_counter_into_label(counter, (api_option_point){0});
	api_label_free(&label);
	return false;
}

/* Bytes as Rust's `escape_ascii` prints those of a path. */
static void print_escaped(const char *before, api_buf buf, const char *after) {
	printf("%s", before);
	for (size_t i = 0; i < buf.len; i++) {
		uint8_t b = buf.ptr[i];
		switch (b) {
		case '\t':
			printf("\\t");
			break;
		case '\r':
			printf("\\r");
			break;
		case '\n':
			printf("\\n");
			break;
		case '\\':
		case '\'':
		case '"':
			printf("\\%c", b);
			break;
		default:
			if (b >= 0x20 && b < 0x7f) {
				putchar(b);
			} else {
				printf("\\x%02x", b);
			}
		}
	}
	printf("%s", after);
	api_buf_free(buf);
}

/* A socket address, as Rust's Display prints the ones the script makes. */
static void print_addr(api_socket_addr addr) {
	if (!addr.v6) {
		printf("%u.%u.%u.%u:%u", addr.ip[0], addr.ip[1], addr.ip[2], addr.ip[3], addr.port);
		return;
	}
	/* Only the loopback, `::1`. */
	printf("[::%u%%%u]:%u", addr.ip[15], addr.scope_id, addr.port);
}

/* A bound, as Rust's Debug prints it. */
static void print_bound(api_bound_u64 bound) {
	switch (bound.kind) {
	case API_BOUND_INCLUDED:
		printf("Included(%llu)", (unsigned long long)bound.value);
		break;
	case API_BOUND_EXCLUDED:
		printf("Excluded(%llu)", (unsigned long long)bound.value);
		break;
	case API_BOUND_UNBOUNDED:
		printf("Unbounded");
		break;
	}
}

static int script(void) {
	printf("limit %llu\n", (unsigned long long)API_LIMIT);

	api_point start = {.x = 1, .y = 2};
	api_point p = api_point_offset(&start, 2, 3);
	printf("offset %d %d\n", p.x, p.y);
	print("debug ", api_point_debug(&p), "\n");
	api_point expected = {.x = 3, .y = 5};
	printf("eq %s\n", api_point_eq(&p, &expected) ? "true" : "false");
	api_point d = api_point_default();
	printf("default %d %d\n", d.x, d.y);

	api_counter *c = api_counter_new(str("c"), 5);
	printf("get %llu\n", (unsigned long long)api_counter_get(c));
	print("name ", api_counter_name(c), "\n");
	uint64_t v;
	api_error err;
	if (api_counter_add(c, 10, &v, &err)) {
		printf("add ok %llu\n", (unsigned long long)v);
	}
	if (!api_counter_add(c, 2000, &v, &err)) {
		print("add err ", api_error_to_string(&err), " ");
		print("", api_error_debug(&err), "\n");
		api_error_free(&err);
	}
	if (!api_counter_sub(c, 20, &v, &err)) {
		printf("sub err underflow by %llu ", (unsigned long long)err.underflow.by);
		print("", api_error_debug(&err), "\n");
		api_error_free(&err);
	}
	api_counter_rename(c, str("d"));
	print("name ", api_counter_name(c), "\n");
	printf("is d %s\n", api_counter_is(c, str("d")) ? "true" : "false");
	api_counter *copy = api_counter_clone(c);
	api_counter_add(copy, 1, NULL, NULL);
	printf("clone %llu original %llu\n", (unsigned long long)api_counter_get(copy),
		   (unsigned long long)api_counter_get(c));
	api_counter_free(copy);
	print("debug ", api_counter_debug(c), "\n");
	api_label label = api_counter_into_label(c, (api_option_point){.some = true, .value = p});
	api_counter_free(c);
	print("label ", api_label_debug(&label), "\n");
	api_label_free(&label);
	/* Empty text is not none: only a null pointer is. */
	api_note empty = {.text = {.ptr = (const uint8_t *)"", .len = 0}};
	printf("note %s %s\n", api_has_text(empty) ? "true" : "false", api_has_text((api_note){0}) ? "true" : "false");

	/* A tuple returns through one out pointer per element. */
	api_counter *whole = api_counter_new(str("s"), 7);
	api_counter *below, *above;
	api_counter_split(whole, 5, &below, &above);
	api_counter_free(whole);
	print("split ", api_counter_name(below), " ");
	printf("%llu ", (unsigned long long)api_counter_get(below));
	print("", api_counter_name(above), " ");
	printf("%llu\n", (unsigned long long)api_counter_get(above));
	api_counter_free(below);
	api_counter_free(above);
	api_counter *one = api_counter_new(str("o"), 3);
	api_counter *only;
	api_only(one, &only);
	api_counter_free(one);
	print("only ", api_counter_name(only), " ");
	printf("%llu\n", (unsigned long long)api_counter_get(only));
	api_counter_free(only);
	uint8_t w8;
	int16_t w16;
	uint32_t w32;
	uint64_t w64;
	api_widths(98305, &w8, &w16, &w32, &w64);
	printf("widths %u %d %u %llu\n", w8, w16, w32, (unsigned long long)w64);
	uint64_t divisors[] = {2, 0};
	for (int i = 0; i < 2; i++) {
		uint64_t q, r;
		if (api_divide(7, divisors[i], &q, &r, &err)) {
			printf("divide %llu %llu\n", (unsigned long long)q, (unsigned long long)r);
		} else {
			print("divide err ", api_error_to_string(&err), "\n");
			api_error_free(&err);
		}
	}
	/* A tuple inside another type is a struct, its fields _0, _1, and so on. */
	uint64_t spreads[] = {98305, 0};
	for (int i = 0; i < 2; i++) {
		api_option_tuple_u8_i16_u32_u64 spread = api_spread(spreads[i]);
		if (spread.some) {
			printf("spread %u %d %u %llu\n", spread.value._0, spread.value._1, spread.value._2,
				   (unsigned long long)spread.value._3);
		} else {
			printf("spread none\n");
		}
	}
	const char *needles[] = {"lo", "x"};
	for (int i = 0; i < 2; i++) {
		api_option_tuple_usize_usize found = api_find(str("hello"), str(needles[i]));
		if (found.some) {
			printf("find %zu %zu\n", found.value._0, found.value._1);
		} else {
			printf("find none\n");
		}
	}
	api_point row_points[] = {{.x = 1, .y = 2}, {.x = 3, .y = 3}};
	api_vec_tuple_string_shape_point stat_rows = api_rows((api_vec_point){.ptr = row_points, .len = 2});
	printf("rows");
	for (size_t i = 0; i < stat_rows.len; i++) {
		/* The array's free frees each element's buffer. */
		printf("%s%.*s ", i == 0 ? " " : ", ", (int)stat_rows.ptr[i]._0.len, (const char *)stat_rows.ptr[i]._0.ptr);
		print("", api_shape_debug(&stat_rows.ptr[i]._1), " ");
		printf("%d %d", stat_rows.ptr[i]._2.x, stat_rows.ptr[i]._2.y);
	}
	printf("\n");
	api_vec_tuple_string_shape_point_free(stat_rows);
	/* Each handle in a tuple moves in, as one taken by value does. */
	api_tuple_string_counter tallies[] = {
		{._0 = str("low"), ._1 = api_counter_new(str("l"), 1)},
		{._0 = str("high"), ._1 = api_counter_new(str("h"), 9)},
	};
	api_board *board = api_board_new((api_vec_tuple_string_counter){.ptr = tallies, .len = 2});
	api_counter_free(tallies[0]._1);
	api_counter_free(tallies[1]._1);
	api_vec_tuple_string_counter ranked = api_board_ranked(board);
	printf("ranked");
	for (size_t i = 0; i < ranked.len; i++) {
		printf("%s%.*s ", i == 0 ? " " : ", ", (int)ranked.ptr[i]._0.len, (const char *)ranked.ptr[i]._0.ptr);
		print("", api_counter_name(ranked.ptr[i]._1), " ");
		printf("%llu", (unsigned long long)api_counter_get(ranked.ptr[i]._1));
	}
	printf("\n");
	api_vec_tuple_string_counter_free(ranked);
	api_board_free(board);
	api_span span = {
		.bounds = {._0 = 2, ._1 = 10},
		.named = {.some = true, .value = {._0 = str("s"), ._1 = API_SHAPE_SQUARE}},
	};
	api_tuple_u32_u32 lower_half, upper_half;
	api_halves(span, &lower_half, &upper_half);
	printf("halves %u %u %u %u ", lower_half._0, lower_half._1, upper_half._0, upper_half._1);
	print("", api_span_debug(&span), "\n");
	uint32_t swapped_n;
	api_buf swapped_text;
	api_swapped((api_tuple_string_u32){._0 = str("t"), ._1 = 4}, &swapped_n, &swapped_text);
	printf("swapped %u ", swapped_n);
	print("", swapped_text, "\n");
	api_option_tuple_option_u32 doubles[] = {
		{.some = false},
		{.some = true, .value = {._0 = {.some = false}}},
		{.some = true, .value = {._0 = {.some = true, .value = 3}}},
	};
	for (int i = 0; i < 3; i++) {
		api_option_tuple_option_u32 doubled = api_doubled(doubles[i]);
		if (!doubled.some) {
			printf("doubled None\n");
		} else if (!doubled.value._0.some) {
			printf("doubled Some((None,))\n");
		} else {
			printf("doubled Some((Some(%u),))\n", doubled.value._0.value);
		}
	}
	/* A borrowed or boxed tuple of one is a tuple struct, not its element. */
	uint8_t lent_bytes[] = {1, 2};
	uint8_t boxed_bytes[] = {3};
	api_tuple_bytes lent_one = api_lent_back((api_tuple_bytes){._0 = {.ptr = lent_bytes, .len = 2}});
	api_tuple_bytes boxed_one = api_boxed_tuple((api_buf){.ptr = boxed_bytes, .len = 1});
	api_buf both[] = {lent_one._0, boxed_one._0};
	for (int i = 0; i < 2; i++) {
		printf("%s[", i == 0 ? "lent back " : " boxed ");
		for (size_t j = 0; j < both[i].len; j++) {
			printf("%s%u", j == 0 ? "" : ", ", both[i].ptr[j]);
		}
		printf("]");
		api_buf_free(both[i]);
	}
	printf("\n");
	/* A borrowed string and an owned one share a tuple struct. */
	api_option_tuple_string formal = api_title(true);
	api_option_tuple_string informal = api_title(false);
	api_option_tuple_string named = api_named(str("x"));
	print("title Some((\"", formal.value._0, "\",)) ");
	printf("%s ", informal.some ? "Some" : "None");
	print("named Some((\"", named.value._0, "\",))\n");

	/* Each lent counter is a new handle to a clone. */
	api_registry *registry = api_registry_new(api_counter_new(str("first"), 1));
	printf("last %s\n", api_registry_last(registry) != NULL ? "true" : "false");
	api_registry_push(registry, api_counter_new(str("last"), 2));
	api_counter *first = api_registry_first(registry);
	api_counter_add(first, 10, NULL, NULL);
	api_counter *lent = api_registry_first(registry);
	printf("first %llu lent %llu\n", (unsigned long long)api_counter_get(lent),
		   (unsigned long long)api_counter_get(first));
	api_counter_free(lent);
	api_counter_free(first);
	api_counter *last = api_registry_last(registry);
	api_counter_add(last, 10, NULL, NULL);
	lent = api_registry_last(registry);
	printf("last %llu lent %llu\n", (unsigned long long)api_counter_get(lent),
		   (unsigned long long)api_counter_get(last));
	api_counter_free(lent);
	api_counter_free(last);
	uint64_t by = 5;
	bool edited = api_registry_edit(registry, edit, &by);
	first = api_registry_first(registry);
	printf("edit %s %llu\n", edited ? "true" : "false", (unsigned long long)api_counter_get(first));
	api_counter_free(first);
	/* A setter moves an object in, as assignment does, leaving the handle to be freed. */
	api_counter *second = api_counter_new(str("second"), 3);
	api_registry_set_first(registry, second);
	api_counter_free(second);
	first = api_registry_first(registry);
	printf("set first %llu ", (unsigned long long)api_counter_get(first));
	print("", api_counter_name(first), "\n");
	api_counter_free(first);
	printf("walk %llu\n", (unsigned long long)api_walk(3, visit, NULL));
	printf("each %u\n", api_each(2, print_visit, NULL));
	printf("offset %u\n", api_offset(seven, NULL, 3));
	printf("shift %u\n", api_shift(seven, NULL, 4));
	api_registry_free(registry);

	/* A type generic only over lifetimes is a handle to its 'static form. */
	api_name *name = api_name_owned(str("a"));
	api_name *b = api_name_owned(str("b"));
	api_name *joined = api_name_join(name, b);
	api_name_free(b);
	print("joined ", api_name_as_str(joined), " ");
	print("", api_name_debug(joined), " ");
	printf("%s\n", api_name_eq(joined, name) ? "true" : "false");
	api_name_free(name);
	char text[] = "c";
	api_name_rename(joined, str(text));
	/* The name took a copy: the caller's buffer is its own again. */
	text[0] = 'x';
	print("renamed ", api_name_as_str(joined), "\n");
	api_name *dd = api_name_owned(str("dd"));
	api_name *longest = api_longer(joined, dd);
	print("longer ", api_name_as_str(longest), "\n");
	api_name_free(longest);
	api_name_free(dd);
	api_name_free(joined);
	api_name *unnamed = api_unnamed();
	print("unnamed [", api_name_as_str(unnamed), "]\n");
	api_name_free(unnamed);

	printf("total %llu\n", (unsigned long long)api_total((api_range_u64){.start = 2, .end = 5}));
	api_range_u64 wide = api_widen((api_range_u64){.start = 2, .end = 5}, 1);
	printf("widen %llu %llu\n", (unsigned long long)wide.start, (unsigned long long)wide.end);
	api_bounds_u64 ranges[] = {
		{.start = {API_BOUND_INCLUDED, 2}, .end = {API_BOUND_EXCLUDED, 5}},
		{.start = {API_BOUND_INCLUDED, 2}, .end = {API_BOUND_INCLUDED, 5}},
		{.start = {API_BOUND_INCLUDED, 3}},
		{.start = {API_BOUND_UNBOUNDED, 0}},
		{.start = {API_BOUND_EXCLUDED, 2}, .end = {API_BOUND_INCLUDED, 4}},
	};
	printf("count");
	for (int i = 0; i < 5; i++) {
		printf(" %zu", api_count(ranges[i]));
	}
	printf("\n");
	printf("end ");
	print_bound(api_end(3));
	printf(" ");
	print_bound(api_end(0));
	printf("\n");
	api_bound_u64 ends_start, ends_end;
	api_ends(3, &ends_start, &ends_end);
	printf("ends ");
	print_bound(ends_start);
	printf(" ");
	print_bound(ends_end);
	printf("\n");
	uint64_t belows[] = {2, 5};
	for (int i = 0; i < 2; i++) {
		uint64_t n;
		api_bounds_u64 outside;
		if (api_below(belows[i], 3, &n, &outside)) {
			printf("below %llu\n", (unsigned long long)n);
		} else {
			printf("below err ");
			print_bound(outside.start);
			printf(" ");
			print_bound(outside.end);
			printf("\n");
		}
	}
	api_window window = {.range = {.start = 2, .end = 8}, .end = api_end(5)};
	printf("window %s %s ", api_window_contains(&window, 4) ? "true" : "false",
		   api_window_contains(&window, 6) ? "true" : "false");
	print("", api_window_debug(&window), "\n");

	print("", api_greet(str("x"), API_SHAPE_SQUARE), "\n");
	printf("len %zu\n", api_len(str("abc")));
	api_buf binary = {.ptr = (const uint8_t *)"\xff\0a", .len = 3};
	print("hex ", api_hex(binary), "\n");
	print("timed hex ", api_timed_hex(binary), "\n");
	print("sample hex ", api_sample_hex(binary), "\n");
	print("queued hex ", api_queued_hex(binary), "\n");
	print("tick ", api_tick_name((api_label){.name = str("tick")}), "\n");
	print("rooted ", api_rooted_name((api_label){.name = str("rooted")}), "\n");
	print("stored ", api_stored_text(str("stored")), "\n");
	print("layer ", api_layer_name((api_label){.name = str("layer")}), "\n");
	api_event events[3] = {
		{.tag = API_EVENT_MOVED, .moved = {._0 = p}},
		{.tag = API_EVENT_NAMED, .named = {.name = str("n")}},
		{.tag = API_EVENT_DONE},
	};
	for (int i = 0; i < 3; i++) {
		print("", api_describe(events[i]), "\n");
	}

	/* A `Copy` handle passed by value is copied, so it stays usable. */
	api_stamp *stamp = api_stamp_from_millis(2500);
	printf("secs %llu\n", (unsigned long long)api_stamp_secs(stamp));
	api_stamp *later = api_stamp_from_millis(3000);
	printf("later %s\n", api_stamp_later(later, stamp) ? "true" : "false");
	api_stamp_free(later);
	printf("secs %llu\n", (unsigned long long)api_stamp_secs(stamp));
	api_message *message = api_message_new(stamp, str("body"));
	api_stamp_free(stamp);
	print("payload ", api_message_payload(message), "\n");
	api_stamp *read = api_message_stamp(message);
	print("stamp ", api_stamp_debug(read), "\n");
	api_stamp_free(read);
	printf("sent %s\n", api_message_sent(message) ? "true" : "false");
	/* Setters assign in place: the bytes are copied in, and a `Copy` handle copied. */
	api_stamp *reset = api_stamp_from_millis(4000);
	api_message_set_stamp(message, reset);
	api_message_set_payload(message, str("edited"));
	print("payload ", api_message_payload(message), "\n");
	read = api_message_stamp(message);
	print("stamp ", api_stamp_debug(read), "\n");
	api_stamp_free(read);
	printf("secs %llu\n", (unsigned long long)api_stamp_secs(reset));
	api_stamp_free(reset);
	api_message_set_stamp(message, NULL);
	printf("stamp %s\n", api_message_stamp(message) != NULL ? "true" : "false");
	api_message_free(message);

	static const api_log_vtable print_log = {.free = log_free, .log = log_line};
	uint8_t levels[] = {1, 2, 3, 4};
	api_buf lines = {.ptr = levels, .len = sizeof levels};
	printf("kept %u\n", api_emit((api_log){.vtable = &print_log}, lines));
	api_logger *logger = api_logger_new((api_log){.vtable = &print_log});
	printf("write %s\n", api_logger_write(logger, str("hi")) ? "true" : "false");
	api_logger_free(logger);
	printf("dropped\n");
	uint8_t threshold = 2;
	printf("filtered %u\n", api_filter(lines, keep_line, &threshold));

	api_channel *channel = api_channel_new();
	api_waker waker = {.wake = wake, .drop = drop, .data = NULL};
	api_channel_recv_future *recv = api_channel_recv(channel);
	api_buf payload;
	printf("recv pending %s\n", api_channel_recv_poll(recv, waker, &payload) == API_POLL_PENDING ? "true" : "false");
	api_channel_send(channel, str("hi"));
	printf("woken %d\n", wakes);
	if (api_channel_recv_poll(recv, waker, &payload) == API_POLL_READY) {
		print("recv ", payload, "\n");
	}
	api_channel_recv_future_free(recv);
	api_channel_send(channel, str(""));
	api_channel_recv_nonempty_future *nonempty = api_channel_recv_nonempty(channel);
	switch (api_channel_recv_nonempty_poll(nonempty, waker, &payload, &err)) {
	case API_POLL_READY:
		printf("nonempty %zu\n", payload.len);
		api_buf_free(payload);
		break;
	case API_POLL_ERROR:
		print("nonempty err ", api_error_to_string(&err), "\n");
		api_error_free(&err);
		break;
	default:
		printf("nonempty pending\n");
	}
	api_channel_recv_nonempty_future_free(nonempty);

	/* The send happens at the call: the future is dropped unpolled. */
	api_channel_send_later_future_free(api_channel_send_later(channel, str("eager")));
	recv = api_channel_recv(channel);
	if (api_channel_recv_poll(recv, waker, &payload) == API_POLL_READY) {
		print("recv ", payload, "\n");
	}
	api_channel_recv_future_free(recv);
	api_channel_send(channel, str("sized"));
	api_channel_recv_sized_future *sized = api_channel_recv_sized(channel);
	size_t len;
	if (api_channel_recv_sized_poll(sized, waker, &payload, &len) == API_POLL_READY) {
		print("sized ", payload, " ");
		printf("%zu\n", len);
	}
	api_channel_recv_sized_future_free(sized);
	/* The waker moves into Rust, which drops it once it wakes. */
	api_channel_notify(channel, waker);
	api_channel_send(channel, str("rung"));
	printf("woken %d\n", wakes);
	api_channel_free(channel);

	/* A value that holds itself is an object. */
	api_node *list = api_node_new(1, NULL);
	api_node_push(list, 2);
	printf("node %llu\n", (unsigned long long)api_node_sum(list));
	api_node_free(list);

	api_tally *tally = api_tally_new();
	api_tally_bump(tally);
	uint32_t count = api_tally_bump(tally);
	printf("tally %u %s\n", count, api_tally_eq(tally, tally) ? "true" : "false");
	printf("both %u\n", api_bump_both(tally, tally));
	printf("peek %u\n", api_peek(tally, peek_tally, NULL));
	api_tally_free(tally);

	/* Another export's types, which this export's function takes and returns. */
	api_area *small = api_area_new((api_size){.width = 2, .height = 3});
	api_area *area = api_grow(small, 1);
	api_area_free(small);
	api_size size = api_area_size(area);
	printf("area %u %u %llu\n", size.width, size.height, (unsigned long long)api_area_pixels(area));
	api_area_free(area);

	/* Generic types, each at the instances it infers, named by their arguments. */
	api_pair_u32 pair = {.left = 2, .right = 5};
	api_pair_u32 swapped = api_pair_u32_swap(&pair);
	printf("pair %u %u sum %u ", swapped.left, swapped.right, api_pair_u32_sum(&pair));
	print("", api_pair_u32_debug(&pair), " ");
	printf("%s\n", api_pair_u32_eq(&pair, &swapped) ? "true" : "false");
	api_pair_u32 twins = {.left = 7, .right = 7};
	printf("even %s %s\n", api_pair_u32_even(&pair) ? "true" : "false",
	       api_pair_u32_even(&twins) ? "true" : "false");
	/* An instance renamed alone, beside another of `Pair` that keeps its name. */
	api_size_tag sizes = api_tag((api_size){.width = 1, .height = 2}, 3);
	api_pair_u8_size flipped = api_size_tag_swap(&sizes);
	printf("sized %u %u ", flipped.left, flipped.right.width);
	print("", api_pair_u8_size_debug(&flipped), "\n");
	api_pair_u32 byte = {.left = 258, .right = 9};
	api_pair_u8_size beside = api_pair_u32_beside_size(&byte, sizes.left);
	printf("beside %u ", beside.left);
	print("", api_pair_u8_size_debug(&beside), "\n");
	/* At its default, named as the type itself. */
	api_stack *stack = api_stack_new();
	const char *names[] = {"a", "b"};
	for (int i = 0; i < 2; i++) {
		api_counter *pushed = api_counter_new(str(names[i]), (uint64_t)i + 1);
		api_stack_push(stack, pushed);
		api_counter_free(pushed);
	}
	api_counter *top = api_stack_pop(stack);
	printf("stack %zu Some(%llu)", api_stack_depth(stack), (unsigned long long)api_counter_get(top));
	api_counter_free(top);
	api_counter *next = api_stack_pop(stack);
	printf(" %s", next != NULL ? "true" : "false");
	api_counter_free(next);
	printf(" %s\n", api_stack_pop(stack) == NULL ? "true" : "false");
	/* Generic functions and methods, at each implementation of the trait bounding them, by
	   reference, which leaves C its handle. */
	api_counter *from_point = api_counter_seeded_point(str("p"), (api_point){.x = -3, .y = 4});
	api_counter *from_size = api_counter_from_size(str("s"), (api_size){.width = 5, .height = 6});
	api_counter *from_counter = api_counter_seeded_counter(str("c"), from_point);
	/* Two instances whose arguments share a last segment: one takes its module too. */
	api_counter *from_polar = api_counter_seeded_polar_point(str("q"), (api_polar_point){.turns = 1, .distance = 2});
	printf("seeded %llu %llu %llu %llu past %llu seeds %llu\n", (unsigned long long)api_counter_get(from_point),
	       (unsigned long long)api_counter_get(from_size), (unsigned long long)api_counter_get(from_counter),
	       (unsigned long long)api_counter_get(from_polar),
	       (unsigned long long)api_stack_depth_past_point(stack, (api_point){.x = 1, .y = 2}),
	       (unsigned long long)api_seeds_point_size((api_size){.width = 2, .height = 3}));
	api_counter_free(from_point);
	api_counter_free(from_size);
	api_counter_free(from_counter);
	api_counter_free(from_polar);
	api_stack_free(stack);

	/* A trait implemented for `Point` and `&Point` apart: one instance. */
	printf("weigh %u\n", api_weigh_point((api_point){.x = -1, .y = 2}));
	/* Instances that name each other, and one made only by an impl of it. */
	api_outer_inner_u8 *outer = api_outer_inner_u8_new(4);
	printf("cross %u outer %u\n",
	       api_cross((api_left_right_u8){.value = {.value = 1}}, (api_right_left_u8){.value = {.value = 2}}),
	       api_outer_inner_u8_get(outer));
	api_outer_inner_u8_free(outer);
	/* A bound that outlives the call takes a value, and an instance only a constant names. */
	printf("kept %llu boxed %u\n", (unsigned long long)api_kept_point((api_point){.x = 1, .y = -2}), api_boxed().value);
	printf("keep %d\n", api_keep_point((api_point){.x = 1, .y = -2}));
	/* Inferred instances taking a lent closure and a foreign implementation, and one only
	 * its impl of a trait names. */
	api_pair_u32 sides = {.left = 2, .right = 5};
	printf("apply %u\n", api_apply_u32(sides, triple, NULL));
	printf("report %u\n", api_report_u32(sides, (api_log){.vtable = &print_log}));
	printf("wrapped %llu\n", (unsigned long long)api_kept_wrap_u32((api_wrap_u32){.value = 4}));
	printf("keeps %u\n", api_keeps((api_log){.vtable = &print_log}).value);

	/* Its Drop runs when C frees the handle, not before. */
	api_token *token = api_token_new(7);
	printf("token %u dropped %u\n", api_token_id(token), api_dropped());
	api_token_free(token);
	printf("dropped %u\n", api_dropped());

	/* A field and the method of its name, each its own function. */
	api_retry *retry = api_retry_new((api_option_u32){.some = true, .value = 5});
	api_option_u32 limit = api_retry_limit_setting(retry);
	printf("retry %d %u %u %u\n", limit.some, limit.value, api_retry_limit(retry), api_retry_delay(retry));
	/* The renamed field's setter, named as its getter. */
	api_retry_set_limit_setting(retry, (api_option_u32){0});
	printf("retry %u\n", api_retry_limit(retry));
	api_retry_free(retry);
	api_slot_u8 *slot = api_slot_u8_new(200);
	printf("slot %u\n", api_slot_u8_value(slot));
	api_slot_u8_free(slot);
	/* Foreign types: a URL parsed from text, socket addresses, OS paths, and `Vec`s. */
	api_url *base;
	api_buf parse_err;
	if (!api_url_parse(str("https://example.com/a/b"), &base, &parse_err)) {
		return 1;
	}
	print("url ", api_url_to_string(base), " host ");
	api_buf host = api_host(base);
	if (host.ptr != NULL) {
		print("Some(\"", host, "\")\n");
	} else {
		printf("None\n");
	}
	api_url *resolved;
	api_buf resolve_err;
	api_url *base_clone = api_url_clone(base);
	if (api_resolve(base_clone, str("c?d"), &resolved, &resolve_err)) {
		print("resolve ", api_url_to_string(resolved), " ");
		printf("%s\n", api_url_eq(resolved, base) ? "true" : "false");
		api_url_free(resolved);
	} else {
		print("resolve err ", resolve_err, "\n");
	}
	api_url_free(base_clone);
	api_url_free(base);
	api_url *bad;
	if (api_url_parse(str("no scheme"), &bad, &parse_err)) {
		print("parse ", api_url_to_string(bad), "\n");
		api_url_free(bad);
	} else {
		print("parse err ", parse_err, "\n");
	}

	api_socket_addr addrs[2] = {
		{.ip = {127, 0, 0, 1}, .port = 8080},
		{.ip = {[15] = 1}, .port = 443, .v6 = true, .scope_id = 3},
	};
	api_vec_u16 ports = api_ports((api_vec_socket_addr){.ptr = addrs, .len = 2});
	printf("ports [%u, %u]\n", ports.ptr[0], ports.ptr[1]);
	api_vec_u16_free(ports);
	api_buf peer_names[] = {str("a"), str("b")};
	api_peer peer_start = {.addr = addrs[1], .names = {.ptr = peer_names, .len = 2}};
	api_peer peer = api_peer_next(&peer_start, str("c"));
	printf("peer Peer { addr: ");
	print_addr(peer.addr);
	printf(", names: [");
	for (size_t i = 0; i < peer.names.len; i++) {
		printf("%s\"%.*s\"", i == 0 ? "" : ", ", (int)peer.names.ptr[i].len, (const char *)peer.names.ptr[i].ptr);
	}
	printf("] } [");
	api_vec_string aliases = api_peer_aliases(&peer);
	for (size_t i = 0; i < aliases.len; i++) {
		printf("%s\"%.*s\"", i == 0 ? "" : ", ", (int)aliases.ptr[i].len, (const char *)aliases.ptr[i].ptr);
	}
	printf("]\n");
	api_vec_string_free(aliases);
	api_buf debug = api_peer_debug(&peer);
	api_peer_free(&peer);
	api_buf_free(debug);

	/* A path's bytes as the OS holds them, which need not be UTF-8. */
	api_buf stray = api_stray();
	api_buf in_stray = api_file(stray, str("f"));
	if (in_stray.len != 3 || memcmp(in_stray.ptr, "\xff/f", 3) != 0) {
		return 1;
	}
	api_buf_free(stray);
	api_buf_free(in_stray);
	api_buf file = api_file(str("dir"), str("f\\'\"\t"));
	api_buf paths[] = {file, str("/")};
	api_vec_option_path parents = api_parents((api_vec_path){.ptr = paths, .len = 2});
	print_escaped("file ", file, "\n");
	printf("parents");
	for (size_t i = 0; i < parents.len; i++) {
		if (parents.ptr[i].ptr != NULL) {
			print_escaped(" ", parents.ptr[i], "");
		} else {
			printf(" none");
		}
	}
	printf("\n");
	/* Each element taken: the array alone is freed. */
	parents.len = 0;
	api_vec_option_path_free(parents);

	api_vec_string words = api_words(str(" one two  three "));
	printf("words %zu [", words.len);
	for (size_t i = 0; i < words.len; i++) {
		printf("%s\"%.*s\"", i == 0 ? "" : ", ", (int)words.ptr[i].len, (const char *)words.ptr[i].ptr);
	}
	printf("]\n");
	print("sentence ", api_sentence(words), " ");
	print("", api_sentence((api_vec_string){0}), "\n");
	api_vec_string_free(words);
	/* An empty array that is not None: its pointer is not null. */
	api_buf no_words[1];
	print("sentence ", api_sentence((api_vec_string){.ptr = no_words, .len = 0}), "|\n");

	api_vec_bytes chunks = api_chunks(str("abcde"), 2);
	printf("chunks [");
	for (size_t i = 0; i < chunks.len; i++) {
		printf("%s[", i == 0 ? "" : ", ");
		for (size_t j = 0; j < chunks.ptr[i].len; j++) {
			printf("%s%u", j == 0 ? "" : ", ", chunks.ptr[i].ptr[j]);
		}
		printf("]");
	}
	printf("]\n");
	api_vec_bytes_free(chunks);

	api_buf counter_names[] = {str("x"), str("y")};
	api_vec_counter counters = api_counters((api_vec_string){.ptr = counter_names, .len = 2});
	printf("counters");
	for (size_t i = 0; i < counters.len; i++) {
		print(i == 0 ? " " : ", ", api_counter_name(counters.ptr[i]), " ");
		printf("%llu", (unsigned long long)api_counter_get(counters.ptr[i]));
	}
	printf("\n");
	/* Each counter moves in, leaving its handle only to free. */
	uint64_t worth = api_worth((api_vec_counter){.ptr = counters.ptr, .len = 1}, (api_vec_counter){.ptr = counters.ptr + 1, .len = 1});
	uint64_t none = api_worth((api_vec_counter){0}, (api_vec_counter){.ptr = counters.ptr, .len = 0});
	printf("worth %llu %llu\n", (unsigned long long)worth, (unsigned long long)none);
	api_vec_counter_free(counters);

	uint32_t row0[] = {1, 2};
	uint32_t row2[] = {3};
	api_vec_u32 rows[] = {{.ptr = row0, .len = 2}, {.ptr = row0, .len = 0}, {.ptr = row2, .len = 1}};
	api_vec_u32 sums = api_sums((api_vec_vec_u32){.ptr = rows, .len = 3});
	printf("sums [%u, %u, %u]\n", sums.ptr[0], sums.ptr[1], sums.ptr[2]);
	api_vec_u32_free(sums);
	api_point summands[] = {{.x = 1, .y = 2}, {.x = -4, .y = 6}};
	api_point summed = api_summed((api_vec_point){.ptr = summands, .len = 2});
	api_point none_summed = api_summed((api_vec_point){.ptr = summands, .len = 0});
	printf("summed %d %d %d %d\n", summed.x, summed.y, none_summed.x, none_summed.y);
	api_buf longest_words[] = {str("a"), str("abc")};
	size_t most = api_longest((api_vec_string){.ptr = longest_words, .len = 2});
	size_t no_longest = api_longest((api_vec_string){.ptr = longest_words, .len = 0});
	printf("longest %zu %zu\n", most, no_longest);

	/* JSON crosses as a document, made from text and read back as text. */
	api_json *doc;
	if (!api_json_parse(str("{\"b\": [1, 2.5, null], \"a\": \"\\u00e9\"}"), &doc, &parse_err)) {
		return 1;
	}
	print("json ", api_encode(doc), "\n");
	print("timed text ", api_timed_text(doc), "\n");
	api_json *tagged = api_tagged(doc, str("t"));
	api_json_free(doc);
	print("tagged ", api_json_to_text(tagged), "\n");
	api_json_free(tagged);
	api_json *broken;
	if (api_json_parse(str("{\"a\":"), &broken, &parse_err)) {
		print("json parse ", api_json_to_text(broken), "\n");
		api_json_free(broken);
	} else {
		print("json parse err ", parse_err, "\n");
	}

	api_url *home;
	if (!api_url_parse(str("https://example.com/a/b"), &home, &parse_err)) {
		return 1;
	}
	api_site *site = api_site_new(home);
	api_url *home0 = api_site_home(site);
	api_url *home1 = api_site_home(site);
	print("site ", api_url_to_string(home0), " ");
	print("", api_url_to_string(home1), "\n");
	api_url_free(home0);
	api_url_free(home1);
	api_site_free(site);
	api_url_free(home);

	/* Slices, passed in and returned as clones, and the socket address of each family. */
	api_vec_shape shapes = api_shape_all();
	printf("shapes [");
	for (size_t i = 0; i < shapes.len; i++) {
		print(i == 0 ? "" : ", ", api_shape_debug(&shapes.ptr[i]), "");
	}
	api_vec_shape_free(shapes);
	api_vec_string greetings = api_greetings();
	printf("] greetings [");
	for (size_t i = 0; i < greetings.len; i++) {
		printf("%s\"%.*s\"", i == 0 ? "" : ", ", (int)greetings.ptr[i].len, (const char *)greetings.ptr[i].ptr);
	}
	printf("]\n");
	api_vec_string_free(greetings);
	api_point trail_points[] = {{.x = 1, .y = 2}, {.x = 3, .y = 4}};
	api_buf trail_names[] = {str("a"), str("b")};
	api_trail *trail =
		api_trail_new((api_vec_point){.ptr = trail_points, .len = 2}, (api_vec_string){.ptr = trail_names, .len = 2});
	api_vec_point points = api_trail_points(trail);
	printf("trail [");
	for (size_t i = 0; i < points.len; i++) {
		print(i == 0 ? "" : ", ", api_point_debug(&points.ptr[i]), "");
	}
	api_vec_point_free(points);
	api_vec_label labels = api_trail_labels(trail);
	printf("] [");
	for (size_t i = 0; i < labels.len; i++) {
		print(i == 0 ? "" : ", ", api_label_debug(&labels.ptr[i]), "");
	}
	printf("]\n");
	api_vec_label_free(labels);
	/* Returned iterators, each collected at the call: drained labels are gone after. */
	api_vec_point iter_points = api_trail_iter(trail);
	printf("iterated [");
	for (size_t i = 0; i < iter_points.len; i++) {
		print(i == 0 ? "" : ", ", api_point_debug(&iter_points.ptr[i]), "");
	}
	api_vec_point_free(iter_points);
	api_vec_string iter_names = api_trail_names(trail);
	printf("] [");
	for (size_t i = 0; i < iter_names.len; i++) {
		printf("%s\"%.*s\"", i == 0 ? "" : ", ", (int)iter_names.ptr[i].len, (const char *)iter_names.ptr[i].ptr);
	}
	api_vec_string_free(iter_names);
	api_buf iter_bytes = api_trail_bytes(trail);
	printf("] [");
	for (size_t i = 0; i < iter_bytes.len; i++) {
		printf("%s%u", i == 0 ? "" : ", ", iter_bytes.ptr[i]);
	}
	printf("]\n");
	api_buf_free(iter_bytes);
	api_vec_label drained = api_trail_drain(trail);
	printf("drained [");
	for (size_t i = 0; i < drained.len; i++) {
		print(i == 0 ? "" : ", ", api_label_debug(&drained.ptr[i]), "");
	}
	api_vec_label_free(drained);
	api_vec_label left = api_trail_labels(trail);
	printf("] then [");
	for (size_t i = 0; i < left.len; i++) {
		print(i == 0 ? "" : ", ", api_label_debug(&left.ptr[i]), "");
	}
	printf("]\n");
	api_vec_label_free(left);
	api_trail_free(trail);
	api_buf numbered_names[] = {str("p"), str("q")};
	api_vec_counter numbered = api_numbered((api_vec_string){.ptr = numbered_names, .len = 2});
	printf("numbered");
	for (size_t i = 0; i < numbered.len; i++) {
		print(i == 0 ? " " : ", ", api_counter_name(numbered.ptr[i]), " ");
		printf("%llu", (unsigned long long)api_counter_get(numbered.ptr[i]));
	}
	printf("\n");
	api_vec_counter_free(numbered);
	api_socket_addr at4 = {.ip = {127, 0, 0, 1}, .port = 8080};
	api_socket_addr at6 = {.ip = {[15] = 1}, .port = 443, .v6 = true, .scope_id = 3};
	api_socket_addr next4, next6;
	api_nexts(at4, at6, &next4, &next6);
	printf("nexts %u.%u.%u.%u %u %u %u\n", next4.ip[0], next4.ip[1], next4.ip[2], next4.ip[3], next4.port, next6.port,
		   next6.scope_id);

	/* A returned `Vec<&Token>` holds clones, which its free drops. */
	api_token *twice = api_token_new(9);
	uint32_t before = api_dropped();
	api_vec_token tokens = api_token_twice(twice);
	if (tokens.len != 2 || api_token_id(tokens.ptr[1]) != 9) {
		return 1;
	}
	api_vec_token_free(tokens);
	if (api_dropped() != before + 2) {
		return 1;
	}
	api_token_free(twice);

	/* Each future dropped its waker once it finished, and Rust the one it was given. */
	return drops == 5 ? 0 : 1;
}

/* Each breaks a contract, which must abort with a message naming the function. */
static int broken(const char *which) {
	if (strcmp(which, "moved") == 0) {
		api_counter *c = api_counter_new(str("c"), 0);
		api_label label = api_counter_into_label(c, (api_option_point){0});
		api_label_free(&label);
		api_counter_get(c);
	} else if (strcmp(which, "borrowed") == 0) {
		api_channel *channel = api_channel_new();
		api_channel_recv_future *recv = api_channel_recv(channel);
		(void)recv;
		api_channel_clear(channel);
	} else if (strcmp(which, "utf8") == 0) {
		api_buf bad = {.ptr = (const uint8_t *)"\xff", .len = 1};
		api_greet(bad, API_SHAPE_CIRCLE);
	} else if (strcmp(which, "finished") == 0) {
		api_channel *channel = api_channel_new();
		api_channel_send(channel, str("x"));
		api_channel_recv_future *recv = api_channel_recv(channel);
		api_waker waker = {0};
		api_buf payload;
		api_channel_recv_poll(recv, waker, &payload);
		api_buf_free(payload);
		api_channel_recv_poll(recv, waker, &payload);
	} else if (strcmp(which, "family") == 0) {
		api_socket_addr v6 = {.ip = {[15] = 1}, .port = 443, .v6 = true};
		api_socket_addr next4, next6;
		api_nexts(v6, v6, &next4, &next6);
	} else if (strcmp(which, "enum") == 0) {
		api_greet(str("x"), (api_shape)2);
	} else if (strcmp(which, "lent") == 0 || strcmp(which, "lent moved") == 0) {
		api_registry *registry = api_registry_new(api_counter_new(str("c"), 0));
		api_registry_edit(registry, strcmp(which, "lent") == 0 ? free_lent : move_lent, NULL);
	} else if (strcmp(which, "null") == 0) {
		api_registry *registry = api_registry_new(api_counter_new(str("c"), 0));
		api_registry_edit(registry, NULL, NULL);
	} else if (strcmp(which, "bound") == 0) {
		api_count((api_bounds_u64){.start = {.kind = (api_bound)3}});
	} else if (strcmp(which, "socket") == 0) {
		api_socket_addr addr = {.ip = {127, 0, 0, 1, 1}};
		api_ports((api_vec_socket_addr){.ptr = &addr, .len = 1});
	} else if (strcmp(which, "array") == 0) {
		api_vec_u16 ports = api_ports((api_vec_socket_addr){0});
		ports.len = 1;
		api_vec_u16_free(ports);
	}
	return 0;
}

int main(int argc, char **argv) {
	if (api_contract() != API_CONTRACT) {
		fprintf(stderr, "api.h and the library are from different builds\n");
		return 1;
	}
	if (argc > 1) {
		return broken(argv[1]);
	}
	return script();
}
