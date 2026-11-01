/* moq's exported C API against the real library: a broadcast's track, published and
 * subscribed to in one process, with every read polled as a future, and an origin.
 * moq-pattern, kio, moq-tokio, hang, and moq-mux export into the same library, and
 * each names the others' types, the catalog is lent to C for a call, and a session's
 * config is filled in field by field.
 *
 * The local reads need no runtime: the waker counts wakes, and each poll that must wait
 * is polled again after the write that wakes it. A session over QUIC runs on moq-tokio's
 * fallback runtime, whose threads wake this one. */

#define _POSIX_C_SOURCE 200809L

#include <errno.h>
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#include "moq.h"

static int failures = 0;

#define CHECK(cond)                                                                                                    \
	do {                                                                                                               \
		if (!(cond)) {                                                                                                 \
			fprintf(stderr, "%s:%d: %s\n", __FILE__, __LINE__, #cond);                                                 \
			failures++;                                                                                                \
		}                                                                                                              \
	} while (0)

static moq_buf str(const char *s) {
	return (moq_buf){.ptr = (const uint8_t *)s, .len = strlen(s)};
}

/* Whether a returned buffer holds `s`, freeing it. */
static int is(moq_buf buf, const char *s) {
	int same = buf.len == strlen(s) && memcmp(buf.ptr, s, buf.len) == 0;
	if (!same) {
		fprintf(stderr, "got \"%.*s\", want \"%s\"\n", (int)buf.len, (const char *)buf.ptr, s);
	}
	moq_buf_free(buf);
	return same;
}

/* Moves the stats track in data into the lent catalog, in place. */
static void sets_stats(void *data, moq_hang_catalog_extra *catalog) {
	moq_hang_catalog_extra_set_stats(catalog, data);
}

/* Whether the lent catalog lists its stats track as `stats`: counts it in data. */
static void lists_stats(void *data, moq_hang_catalog_extra *catalog) {
	moq_catalog_stats *stats = moq_hang_catalog_extra_stats(catalog);
	moq_catalog_stats *want = moq_catalog_stats_new(str("stats"));
	if (stats != NULL && moq_catalog_stats_eq(stats, want)) {
		(*(int *)data)++;
	}
	moq_catalog_stats_free(want);
	moq_catalog_stats_free(stats);
}

/* Whether `buf` holds `s` anywhere. */
static int contains(moq_buf buf, const char *s) {
	size_t len = strlen(s);
	for (size_t i = 0; i + len <= buf.len; i++) {
		if (memcmp(buf.ptr + i, s, len) == 0) {
			return 1;
		}
	}
	return 0;
}

/* Sets the lent catalog's `app` section to the JSON document in data, in place. */
static void sets_app(void *data, moq_hang_catalog_extra *catalog) {
	moq_catalog_hang_extra *sections = moq_hang_catalog_extra_ext(catalog);
	moq_mux_error *err = NULL;
	CHECK(moq_catalog_hang_extra_set(sections, str("app"), data, &err));
	moq_hang_catalog_extra_set_ext(catalog, sections);
	moq_catalog_hang_extra_free(sections);
}

/* What C reads back from the lent catalog: its JSON, and its `app` section as text. */
struct readback {
	moq_buf json;
	moq_buf app;
};

static void reads_back(void *data, moq_hang_catalog_extra *catalog) {
	struct readback *back = data;
	moq_hang_error *hang_err = NULL;
	CHECK(moq_hang_catalog_extra_to_json(catalog, &back->json, &hang_err));
	moq_catalog_hang_extra *sections = moq_hang_catalog_extra_ext(catalog);
	moq_json *app = moq_catalog_hang_extra_get(sections, str("app"));
	if (app != NULL) {
		back->app = moq_json_to_text(app);
		moq_json_free(app);
	}
	moq_catalog_hang_extra_free(sections);
}

static int wakes = 0;

static void wake(void *data) {
	(void)data;
	wakes++;
}

/* A session's futures complete on moq-tokio's runtime threads, so its waker signals this
 * thread, which waits for a wake after a poll finds a future pending. */
static pthread_mutex_t woken_lock = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t woken_cond = PTHREAD_COND_INITIALIZER;
static unsigned long woken = 0;

static void wake_session(void *data) {
	(void)data;
	pthread_mutex_lock(&woken_lock);
	woken++;
	pthread_cond_broadcast(&woken_cond);
	pthread_mutex_unlock(&woken_lock);
}

static const moq_waker session_waker = {.wake = wake_session};

static unsigned long wakes_so_far(void) {
	pthread_mutex_lock(&woken_lock);
	unsigned long n = woken;
	pthread_mutex_unlock(&woken_lock);
	return n;
}

/* Waits for a wake after `seen`, failing the run rather than hanging when none comes. */
static void wait_wake(unsigned long seen) {
	struct timespec deadline;
	clock_gettime(CLOCK_REALTIME, &deadline);
	deadline.tv_sec += 30;
	pthread_mutex_lock(&woken_lock);
	while (woken == seen) {
		if (pthread_cond_timedwait(&woken_cond, &woken_lock, &deadline) == ETIMEDOUT) {
			fprintf(stderr, "no wake in 30 seconds\n");
			abort();
		}
	}
	pthread_mutex_unlock(&woken_lock);
}

/* The server's accept loop, which must keep running while the client waits on it. */
static moq_tokio_server_serve_publish_future *serving = NULL;

/* Polls `poll` until it is ready, and the server's accept loop alongside it. */
#define AWAIT(poll)                                                                                                    \
	for (;;) {                                                                                                         \
		unsigned long seen = wakes_so_far();                                                                           \
		moq_tokio_error *serve_err = NULL;                                                                             \
		if (moq_tokio_server_serve_publish_poll(serving, session_waker, &serve_err) == MOQ_POLL_READY) {             \
			fprintf(stderr, "%s:%d: the server stopped\n", __FILE__, __LINE__);                                        \
			abort();                                                                                                   \
		}                                                                                                              \
		if ((poll) == MOQ_POLL_READY) {                                                                                \
			break;                                                                                                     \
		}                                                                                                              \
		wait_wake(seen);                                                                                               \
	}

int main(void) {
	moq_waker waker = {.wake = wake};
	moq_error *err = NULL;
	CHECK(moq_contract() == MOQ_CONTRACT);

	moq_broadcast_info *info = moq_broadcast_info_new();
	moq_broadcast_producer *broadcast = moq_broadcast_info_produce(info);
	moq_broadcast_info_free(info);

	moq_track_producer *track = NULL;
	CHECK(moq_broadcast_producer_create_track(broadcast, str("video"), NULL, &track, &err));
	CHECK(is(moq_track_producer_name(track), "video"));

	/* A second track of the same name is an error, printed as Rust prints it. */
	moq_track_producer *duplicate = NULL;
	CHECK(!moq_broadcast_producer_create_track(broadcast, str("video"), NULL, &duplicate, &err));
	CHECK(err != NULL && is(moq_error_to_string(err), "duplicate"));
	moq_error_free(err);

	moq_track_consumer *consumer = moq_track_producer_consume(track);
	moq_track_consumer_subscribe_future *subscribing = moq_track_consumer_subscribe(consumer, NULL);
	moq_track_subscriber *subscriber = NULL;
	CHECK(moq_track_consumer_subscribe_poll(subscribing, waker, &subscriber, &err) == MOQ_POLL_READY);
	moq_track_consumer_subscribe_future_free(subscribing);

	/* A group's arrival wakes the subscriber's pending receive. */
	moq_track_subscriber_recv_group_future *receiving = moq_track_subscriber_recv_group(subscriber);
	moq_group_consumer *group = NULL;
	CHECK(moq_track_subscriber_recv_group_poll(receiving, waker, &group, &err) == MOQ_POLL_PENDING);
	moq_group_producer *writer = NULL;
	CHECK(moq_track_producer_append_group(track, &writer, &err));
	CHECK(wakes == 1);
	CHECK(moq_track_subscriber_recv_group_poll(receiving, waker, &group, &err) == MOQ_POLL_READY);
	moq_track_subscriber_recv_group_future_free(receiving);
	CHECK(group != NULL && moq_group_consumer_index(group) == 0);

	/* So does a frame's. */
	moq_group_consumer_read_frame_future *reading = moq_group_consumer_read_frame(group);
	moq_frame_frame *frame = NULL;
	CHECK(moq_group_consumer_read_frame_poll(reading, waker, &frame, &err) == MOQ_POLL_PENDING);
	CHECK(moq_group_producer_write_frame(writer, NULL, str("hello"), &err));
	CHECK(wakes == 2);
	CHECK(moq_group_consumer_read_frame_poll(reading, waker, &frame, &err) == MOQ_POLL_READY);
	moq_group_consumer_read_frame_future_free(reading);
	CHECK(frame != NULL && is(moq_frame_frame_payload(frame), "hello"));
	CHECK(moq_frame_frame_timestamp(frame) == NULL);
	/* When it arrived: kio's clock's `Instant`, from kio's own export. */
	moq_clock_instant *arrived = moq_frame_frame_arrived(frame);
	moq_clock_instant *now = moq_clock_now();
	CHECK(moq_clock_instant_checked_duration_since(now, arrived).some);
	moq_clock_instant_free(now);
	moq_clock_instant_free(arrived);
	moq_frame_frame_free(frame);

	/* A timestamp is a `Copy` value behind a handle: passing it copies it out. The track
	 * is untimed, so a timestamped frame is refused. */
	moq_timestamp *at = NULL;
	moq_time_overflow *overflow = NULL;
	CHECK(moq_timestamp_from_millis(1500, &at, &overflow));
	CHECK(moq_timestamp_as_secs(at) == 1);
	CHECK(!moq_group_producer_write_frame(writer, at, str("late"), &err));
	CHECK(err != NULL && is(moq_error_to_string(err), "frame timestamp doesn't fit its track"));
	moq_error_free(err);
	CHECK(moq_timestamp_as_secs(at) == 1);

	/* The last frame, bytes that are not UTF-8, then the group's end, which reads as None. */
	CHECK(moq_group_producer_write_frame(writer, NULL, str("\xffworld"), &err));
	CHECK(moq_group_producer_finish(writer, &err));
	reading = moq_group_consumer_read_frame(group);
	CHECK(moq_group_consumer_read_frame_poll(reading, waker, &frame, &err) == MOQ_POLL_READY);
	moq_group_consumer_read_frame_future_free(reading);
	CHECK(frame != NULL && is(moq_frame_frame_payload(frame), "\xffworld"));
	moq_frame_frame_free(frame);
	reading = moq_group_consumer_read_frame(group);
	frame = (moq_frame_frame *)1;
	CHECK(moq_group_consumer_read_frame_poll(reading, waker, &frame, &err) == MOQ_POLL_READY);
	CHECK(frame == NULL);
	moq_group_consumer_read_frame_future_free(reading);

	moq_timestamp_free(at);
	moq_group_consumer_free(group);
	moq_group_producer_free(writer);
	moq_track_subscriber_free(subscriber);
	moq_track_consumer_free(consumer);
	moq_track_producer_free(track);
	moq_broadcast_producer_free(broadcast);

	/* An origin and the driver that runs it, a tuple returned through two out pointers.
	 * Its root, which it lends, is returned as a new handle to a clone. */
	moq_origin_config *config = moq_origin_config_default();
	moq_origin_producer *origin = NULL;
	moq_origin_driver *driver = NULL;
	moq_origin_producer_new(config, &origin, &driver);
	moq_origin_config_free(config);
	CHECK(origin != NULL && driver != NULL);
	moq_path *root = moq_origin_producer_root(origin);
	CHECK(is(moq_path_as_str(root), ""));
	moq_path_free(root);

	/* Scoped beneath a root to patterns, which moq-pattern's own export builds. */
	moq_pattern *alice = NULL;
	moq_invalid_pattern invalid;
	CHECK(moq_pattern_subtree(str("alice"), &alice, &invalid));
	moq_patterns *patterns = moq_patterns_new();
	CHECK(moq_patterns_insert(patterns, alice));
	moq_pattern_free(alice);
	moq_origin_producer *scoped = NULL;
	CHECK(moq_origin_producer_scope(origin, str("room"), patterns, &scoped, &err));
	moq_patterns_free(patterns);
	root = moq_origin_producer_root(scoped);
	CHECK(is(moq_path_as_str(root), "room"));
	moq_path_free(root);
	moq_path *absolute = NULL;
	CHECK(moq_origin_producer_absolute(scoped, str("alice"), &absolute, &err));
	CHECK(is(moq_path_as_str(absolute), "room/alice"));
	moq_path_free(absolute);
	moq_patterns *allowed = moq_origin_producer_allowed(scoped);
	CHECK(moq_patterns_matches(allowed, str("alice/cam")));
	CHECK(!moq_patterns_matches(allowed, str("bob")));
	moq_patterns_free(allowed);
	moq_origin_consumer *reader = moq_origin_producer_consume(scoped);
	CHECK(moq_origin_consumer_absolute(reader, str("bob"), &absolute, &err));
	CHECK(is(moq_path_as_str(absolute), "room/bob"));
	moq_path_free(absolute);
	moq_origin_consumer_free(reader);
	moq_origin_producer_free(scoped);

	/* A broadcast the origin creates is stamped with its path, which its info lends. */
	moq_origin_publication *publication = NULL;
	CHECK(moq_origin_producer_create_broadcast(origin, str("room/alice"), &publication, &err));
	moq_broadcast_producer *created = moq_origin_publication_broadcast(publication);
	moq_origin_publication_free(publication);
	moq_broadcast_info *stamped = moq_broadcast_producer_info(created);
	moq_path *path = moq_broadcast_info_path(stamped);
	CHECK(is(moq_path_as_str(path), "room/alice"));

	/* Paths own their segments, so C builds them, and relative ones, from a string. */
	moq_path *cam = NULL;
	moq_path_error path_error;
	CHECK(!moq_path_new(str("/cam/"), &cam, &path_error) && path_error == MOQ_PATH_ERROR_EMPTY_SEGMENT);
	CHECK(moq_path_new(str("cam"), &cam, &path_error));
	CHECK(is(moq_path_as_str(cam), "cam"));
	moq_path *joined = NULL;
	CHECK(moq_path_join(path, cam, &joined, &path_error));
	CHECK(is(moq_path_as_str(joined), "room/alice/cam"));
	moq_path *stripped = moq_path_strip_prefix(joined, path);
	CHECK(stripped != NULL && moq_path_eq(stripped, cam));
	moq_path_free(stripped);
	CHECK(moq_path_strip_prefix(path, joined) == NULL);
	moq_relative *up = NULL;
	CHECK(moq_relative_new(str("../bob"), &up, &path_error));
	moq_path *resolved = NULL;
	CHECK(moq_path_resolve(joined, up, &resolved, &path_error));
	CHECK(is(moq_path_as_str(resolved), "room/bob"));
	moq_path_free(resolved);
	moq_relative_free(up);
	moq_path_free(joined);
	moq_path_free(cam);
	moq_path_free(path);
	moq_broadcast_info_free(stamped);
	moq_broadcast_producer_free(created);

	/* The driver, polled as a language's event loop would: at the time, and with a waiter
	 * made from the waker and held across polls, since what a poll registers lives only
	 * as long as its waiter. It runs until every owner of the origin is gone. */
	moq_waiter *waiter = moq_waiter_new(waker);
	now = moq_clock_now();
	moq_clock_instant *deadline = NULL;
	CHECK(moq_origin_driver_poll(driver, now, waiter, &deadline, &err));
	moq_clock_instant_free(deadline);
	moq_clock_instant_free(now);
	/* A client publishes the origin through moq-net's generic `with_publisher`, at each
	 * type that implements its bound, by reference, so C keeps its handle. */
	moq_client *client = moq_client_new();
	moq_client *publishing = moq_client_with_publisher_producer(client, origin);
	CHECK(publishing != NULL);
	moq_client_free(publishing);
	moq_client_free(client);
	int before = wakes;
	moq_origin_producer_free(origin);
	/* The last owner's drop wakes the driver, which then finishes, cleanly. */
	CHECK(wakes > before);
	now = moq_clock_now();
	deadline = NULL;
	err = NULL;
	CHECK(!moq_origin_driver_poll(driver, now, waiter, &deadline, &err));
	CHECK(deadline == NULL && err != NULL && is(moq_error_to_string(err), "closed"));
	moq_error_free(err);
	err = NULL;
	moq_clock_instant_free(now);
	moq_waiter_free(waiter);
	moq_origin_driver_free(driver);

	/* moq-mux reads an OpusHead into hang's catalog config: 44.1 kHz in, stereo, which
	 * the catalog lists at the decoder's 48 kHz, keeping the head as its description. */
	static const uint8_t head[] = {'O', 'p', 'u', 's', 'H', 'e', 'a', 'd', 1, 2, 0x38, 0x01, 0x44, 0xac, 0, 0, 0, 0, 0};
	moq_catalog_audio_config *audio = NULL;
	moq_mux_error *mux_err = NULL;
	CHECK(moq_codec_opus_audio_config((moq_buf){.ptr = head, .len = sizeof head}, &audio, &mux_err));
	CHECK(moq_catalog_audio_config_sample_rate(audio) == 48000);
	CHECK(moq_catalog_audio_config_channel_count(audio) == 2);
	moq_catalog_audio_codec codec = moq_catalog_audio_config_codec(audio);
	CHECK(codec.tag == MOQ_CATALOG_AUDIO_CODEC_OPUS);
	moq_catalog_audio_codec_free(&codec);
	moq_buf description = moq_catalog_audio_config_description(audio);
	CHECK(description.len == sizeof head && memcmp(description.ptr, head, sizeof head) == 0);
	moq_buf_free(description);
	moq_catalog_audio_config_free(audio);
	/* A truncated head is moq-mux's error. */
	CHECK(!moq_codec_opus_audio_config((moq_buf){.ptr = head, .len = 8}, &audio, &mux_err));
	CHECK(mux_err != NULL);
	moq_buf message = moq_mux_error_to_string(mux_err);
	CHECK(message.len > 0);
	moq_buf_free(message);
	moq_mux_error_free(mux_err);
	mux_err = NULL;

	/* A subscription's ranges, from any iterable of them: an array, copied in. */
	moq_track_range ranges[] = {
	    moq_track_range_live(),
	    moq_track_range_groups((moq_bounds_u64){.start = {.kind = MOQ_BOUND_INCLUDED, .value = 2},
	                                            .end = {.kind = MOQ_BOUND_EXCLUDED, .value = 6}}),
	};
	moq_track_subscription *unranged = moq_track_subscription_default();
	moq_track_subscription *ranged =
	    moq_track_subscription_with_ranges(unranged, (moq_vec_track_range){.ptr = ranges, .len = 2});
	moq_track_subscription_free(unranged);
	moq_vec_track_range got = moq_track_subscription_ranges(ranged);
	CHECK(got.len == 2 && moq_track_range_eq(&got.ptr[0], &ranges[0]));
	CHECK(got.len == 2 && got.ptr[1].start.tag == MOQ_TRACK_ANCHOR_ABSOLUTE && got.ptr[1].start.absolute._0 == 2);
	CHECK(got.len == 2 && got.ptr[1].end.some && got.ptr[1].end.value.absolute._0 == 6);
	moq_vec_track_range_free(got);
	moq_track_range_free(&ranges[0]);
	moq_track_range_free(&ranges[1]);
	moq_track_subscription_free(ranged);

	/* A catalog at moq-mux's untyped extension, from the config C builds, whose generic
	 * `is_claimed` is named by the last segment of its argument, and whose `mutate` lends
	 * the catalog to C, which sets its fields in place. */
	moq_broadcast_info *listed = moq_broadcast_info_new();
	moq_broadcast_producer *publisher = moq_broadcast_info_produce(listed);
	moq_broadcast_info_free(listed);
	moq_catalog_config *catalog_config = moq_catalog_config_default();
	moq_hang_catalog_extra *hang = moq_hang_catalog_extra_default();
	moq_catalog_config_extra *extra = moq_catalog_config_with_catalog_extra(catalog_config, hang);
	moq_catalog_config_free(catalog_config);
	moq_hang_catalog_extra_free(hang);
	moq_catalog_producer_extra *catalog = NULL;
	CHECK(moq_catalog_producer_extra_new(publisher, extra, &catalog, &mux_err));
	moq_catalog_config_extra_free(extra);
	CHECK(!moq_catalog_producer_extra_is_claimed_binary_config(catalog, str("state")));
	/* A field of the lent catalog, set in place: the next call reads it. */
	moq_catalog_stats *stats = moq_catalog_stats_new(str("stats"));
	CHECK(moq_catalog_producer_extra_mutate(catalog, sets_stats, stats, &mux_err));
	moq_catalog_stats_free(stats);
	int listed_stats = 0;
	CHECK(moq_catalog_producer_extra_mutate(catalog, lists_stats, &listed_stats, &mux_err));
	CHECK(listed_stats == 1);
	/* A JSON track the catalog lists, published through moq-mux's generic `json_snapshot`
	 * at a JSON document, and a section C parses from text, set in the lent catalog and
	 * read back as text. */
	moq_track_producer *status = NULL;
	CHECK(moq_broadcast_producer_create_track(publisher, str("status"), NULL, &status, &err));
	moq_json_config *json_config = moq_json_config_default();
	moq_json_options *json_options = moq_json_options_default();
	moq_json_snapshot_json *status_snapshot = NULL;
	CHECK(moq_catalog_producer_extra_json_snapshot_config(catalog, status, json_config, json_options, &status_snapshot,
	                                                      &mux_err));
	moq_json_config_free(json_config);
	moq_json_options_free(json_options);
	moq_track_producer_free(status);
	CHECK(is(moq_json_snapshot_json_name(status_snapshot), "status"));
	CHECK(moq_catalog_producer_extra_is_claimed_json_config(catalog, str("status")));
	/* A value C parses is published as the track's latest, which a subscriber reads back
	 * as JSON text. */
	moq_json *latest = NULL;
	moq_buf json_err;
	CHECK(moq_json_parse(str("{\"on\": true}"), &latest, &json_err));
	CHECK(moq_json_snapshot_json_update(status_snapshot, latest, &mux_err));
	moq_json_free(latest);
	moq_track_subscriber *status_subscriber = moq_json_snapshot_json_consume(status_snapshot);
	moq_track_subscriber_recv_group_future *status_receiving = moq_track_subscriber_recv_group(status_subscriber);
	moq_group_consumer *status_group = NULL;
	CHECK(moq_track_subscriber_recv_group_poll(status_receiving, waker, &status_group, &err) == MOQ_POLL_READY);
	moq_track_subscriber_recv_group_future_free(status_receiving);
	moq_frame_frame *status_frame = NULL;
	if (status_group != NULL) {
		moq_group_consumer_read_frame_future *status_reading = moq_group_consumer_read_frame(status_group);
		CHECK(moq_group_consumer_read_frame_poll(status_reading, waker, &status_frame, &err) == MOQ_POLL_READY);
		moq_group_consumer_read_frame_future_free(status_reading);
		moq_group_consumer_free(status_group);
	}
	CHECK(status_frame != NULL && is(moq_frame_frame_payload(status_frame), "{\"on\":true}"));
	moq_frame_frame_free(status_frame);
	moq_track_subscriber_free(status_subscriber);
	moq_json *app = NULL;
	CHECK(moq_json_parse(str("{\"v\": [1, true]}"), &app, &json_err));
	CHECK(moq_catalog_producer_extra_mutate(catalog, sets_app, app, &mux_err));
	moq_json_free(app);
	struct readback back = {0};
	CHECK(moq_catalog_producer_extra_mutate(catalog, reads_back, &back, &mux_err));
	CHECK(contains(back.json, "\"status\""));
	moq_buf_free(back.json);
	CHECK(is(back.app, "{\"v\":[1,true]}"));
	/* Malformed text fails to parse, and a reserved section is moq-mux's error. */
	CHECK(!moq_json_parse(str("{\"v\":"), &app, &json_err));
	CHECK(is(json_err, "EOF while parsing a value at line 1 column 5"));
	moq_catalog_hang_extra *sections = moq_catalog_hang_extra_default();
	CHECK(moq_json_parse(str("1"), &app, &json_err));
	CHECK(!moq_catalog_hang_extra_set(sections, str("json"), app, &mux_err));
	CHECK(mux_err != NULL && is(moq_mux_error_to_string(mux_err), "reserved catalog section: json"));
	moq_mux_error_free(mux_err);
	mux_err = NULL;
	moq_json_free(app);
	moq_catalog_hang_extra_free(sections);
	moq_json_snapshot_json_free(status_snapshot);
	moq_catalog_producer_extra_free(catalog);
	moq_broadcast_producer_free(publisher);

	/* Outside any tokio runtime, as C is, moq-tokio runs on the fallback it starts. */
	moq_origin_producer *spawned = moq_origin_spawn();
	moq_origin_producer_free(spawned);
	moq_connect_config *client_config = moq_connect_config_default();
	moq_quic_config *quic = moq_quic_config_default();
	moq_tokio_client *tokio_client = NULL;
	moq_tokio_error *tokio_err = NULL;
	CHECK(moq_connect_config_init(client_config, quic, &tokio_client, &tokio_err));
	moq_connect_config_free(client_config);
	moq_quic_config_free(quic);
	moq_tokio_client_free(tokio_client);

	/* moq-tokio's terminal connect errors, which moq-net's `Error` shares no names with. */
	moq_connect_error unauthorized = MOQ_CONNECT_ERROR_UNAUTHORIZED;
	CHECK(moq_connect_error_is_auth(&unauthorized));
	CHECK(is(moq_connect_error_to_string(&unauthorized), "unauthorized"));

	/* A session's config, filled in as Rust assigns it, and a nested field by getting a copy,
	 * editing it, and setting it back. */
	moq_connect_config *connect = moq_connect_config_default();
	moq_tls_connect *tls = moq_connect_config_tls(connect);
	moq_tls_connect_set_insecure(tls, (moq_option_bool){.some = true, .value = true});
	moq_tls_connect_set_host_name(tls, str("relay.example"));
	/* A `Vec<String>`, an array of buffers C keeps. */
	moq_buf pins[] = {str("ab:cd")};
	moq_tls_connect_set_fingerprint(tls, (moq_vec_string){.ptr = pins, .len = 1});
	moq_connect_config_set_tls(connect, tls);
	moq_tls_connect_free(tls);
	moq_connect_config_set_implementation(connect, str("moq.c"));
	/* A URL, parsed from text, failing with why, and moved in. */
	moq_url *url = NULL;
	moq_buf parse_err;
	CHECK(!moq_url_parse(str("relay.example"), &url, &parse_err));
	CHECK(is(parse_err, "relative URL without a base"));
	CHECK(moq_url_parse(str("https://relay.example:4443/demo"), &url, &parse_err));
	moq_connect_config_set_url(connect, url);
	moq_url_free(url);
	tls = moq_connect_config_tls(connect);
	moq_option_bool insecure = moq_tls_connect_insecure(tls);
	CHECK(insecure.some && insecure.value);
	CHECK(is(moq_tls_connect_host_name(tls), "relay.example"));
	/* TLS files, as OS paths: none by default. */
	moq_vec_path roots = moq_tls_connect_root(tls);
	moq_vec_string fingerprints = moq_tls_connect_fingerprint(tls);
	CHECK(roots.ptr != NULL && roots.len == 0);
	CHECK(fingerprints.len == 1 && fingerprints.ptr[0].len == 5 && memcmp(fingerprints.ptr[0].ptr, "ab:cd", 5) == 0);
	moq_vec_path_free(roots);
	moq_vec_string_free(fingerprints);
	moq_tls_connect_free(tls);
	CHECK(is(moq_connect_config_implementation(connect), "moq.c"));
	url = moq_connect_config_url(connect);
	CHECK(url != NULL && is(moq_url_to_string(url), "https://relay.example:4443/demo"));
	/* The config's URL, dialed with no tokens. */
	moq_auth_tokens *tokens = moq_auth_tokens_new((moq_vec_auth_token){0});
	moq_versions *versions = moq_versions_default();
	moq_auth_dial *dial = moq_auth_tokens_dial(tokens, url, versions);
	moq_url *dialed = moq_auth_dial_url(dial);
	CHECK(moq_url_eq(dialed, url));
	moq_url_free(dialed);
	moq_auth_dial_free(dial);
	moq_auth_tokens_free(tokens);
	moq_url_free(url);
	moq_connect_config_free(connect);

	/* A backoff's fields are settings left unset, and the methods of their names resolve them. */
	moq_backoff *backoff = moq_backoff_default();
	CHECK(moq_backoff_initial_setting(backoff) == NULL);
	moq_duration initial = moq_backoff_initial(backoff);
	CHECK(initial.secs == 1 && initial.nanos == 0);
	moq_backoff_free(backoff);

	/* A `Vec<String>`, each element a buffer the array frees. */
	moq_vec_string alpns = moq_versions_alpns(versions);
	CHECK(alpns.len > 0);
	for (size_t i = 0; i < alpns.len; i++) {
		CHECK(alpns.ptr[i].len > 0);
	}
	moq_vec_string_free(alpns);
	moq_versions_free(versions);
	/* A socket address, crossing in a listener's bind, set on its config and read back. */
	moq_listen_bind bind = {.tag = MOQ_LISTEN_BIND_ADDR, .addr = {._0 = {.ip = {[15] = 1}, .port = 4443, .v6 = true}}};
	CHECK(is(moq_listen_bind_to_string(&bind), "[::1]:4443"));
	moq_listen_bind v4 = {.tag = MOQ_LISTEN_BIND_ADDR, .addr = {._0 = {.ip = {127, 0, 0, 1}, .port = 0}}};
	CHECK(is(moq_listen_bind_to_string(&v4), "127.0.0.1:0"));
	CHECK(!moq_listen_bind_eq(&bind, &v4));
	moq_listen_config *listen = moq_listen_config_default();
	moq_listen_config_set_bind(listen, (moq_option_listen_bind){.some = true, .value = bind});
	moq_option_listen_bind bound = moq_listen_config_bind(listen);
	CHECK(bound.some && moq_listen_bind_eq(&bound.value, &bind));
	moq_listen_bind_free(&bound.value);
	/* The preferred address of each family, which takes only that family. */
	moq_socket_addr preferred = {.ip = {10, 0, 0, 1}, .port = 4443};
	moq_listen_config_set_preferred_v4(listen, (moq_option_socket_addr){.some = true, .value = preferred});
	moq_option_socket_addr preferred_v4 = moq_listen_config_preferred_v4(listen);
	CHECK(preferred_v4.some && !preferred_v4.value.v6 && preferred_v4.value.ip[0] == 10 && preferred_v4.value.port == 4443);
	CHECK(!moq_listen_config_preferred_v6(listen).some);
	moq_listen_config_free(listen);
	/* A constant slice, as an array of clones. */
	moq_vec_string alpns_all = moq_alpns();
	CHECK(alpns_all.len > 0);
	moq_vec_string_free(alpns_all);

	/* A session from C: a server binds QUIC on port 0, reads back the port it got, and
	 * serves an origin with one call, and a client dials it with one call and reads a frame
	 * back. Neither has a runtime of C's, so moq-tokio's fallback runs both. */
	moq_origin_producer *served = moq_origin_spawn();
	moq_origin_publication *live = NULL;
	CHECK(moq_origin_producer_create_broadcast(served, str("live"), &live, &err));
	moq_origin_route *route = moq_origin_route_default();
	CHECK(moq_origin_publication_announce(live, route, &err));
	moq_origin_route_free(route);
	moq_broadcast_producer *live_broadcast = moq_origin_publication_broadcast(live);
	moq_track_producer *live_track = NULL;
	CHECK(moq_broadcast_producer_create_track(live_broadcast, str("video"), NULL, &live_track, &err));
	moq_group_producer *live_group = NULL;
	CHECK(moq_track_producer_append_group(live_track, &live_group, &err));
	CHECK(moq_group_producer_write_frame(live_group, NULL, str("hello"), &err));
	CHECK(moq_group_producer_finish(live_group, &err));
	moq_group_producer_free(live_group);

	moq_server_config *server_config = moq_server_config_default();
	moq_listen_config *listen_config = moq_server_config_listen(server_config);
	moq_listen_bind any = {.tag = MOQ_LISTEN_BIND_ADDR, .addr = {._0 = {.port = 0, .v6 = true}}};
	moq_listen_config_set_bind(listen_config, (moq_option_listen_bind){.some = true, .value = any});
	moq_tls_listen *tls_listen = moq_tls_listen_default();
	moq_buf names[] = {str("localhost")};
	moq_tls_listen_set_generate(tls_listen, (moq_vec_string){.ptr = names, .len = 1});
	moq_listen_config_set_tls(listen_config, tls_listen);
	moq_tls_listen_free(tls_listen);
	moq_server_config_set_listen(server_config, listen_config);
	moq_listen_config_free(listen_config);
	moq_tokio_server *server = NULL;
	CHECK(moq_server_config_init(server_config, &server, &tokio_err));
	moq_server_config_free(server_config);
	moq_socket_addr local;
	CHECK(moq_tokio_server_local_addr(server, &local, &tokio_err));
	CHECK(local.v6 && local.port != 0);
	moq_origin_consumer *published = moq_origin_producer_consume(served);
	serving = moq_tokio_server_serve_publish(server, published);
	moq_tokio_server_free(server);
	moq_origin_consumer_free(published);

	char address[32];
	snprintf(address, sizeof address, "moqt://localhost:%u", (unsigned)local.port);
	moq_connect_config *dialing = moq_connect_config_default();
	CHECK(moq_url_parse(str(address), &url, &parse_err));
	moq_connect_config_set_url(dialing, url);
	moq_url_free(url);
	tls = moq_connect_config_tls(dialing);
	moq_tls_connect_set_insecure(tls, (moq_option_bool){.some = true, .value = true});
	moq_connect_config_set_tls(dialing, tls);
	moq_tls_connect_free(tls);
	moq_client_config *session_client = moq_client_config_default();
	moq_client_config_set_connect(session_client, dialing);
	moq_connect_config_free(dialing);
	moq_tokio_client *dialer = NULL;
	CHECK(moq_client_config_init(session_client, &dialer, &tokio_err));
	moq_client_config_free(session_client);
	moq_tokio_client *dialer_copy = moq_tokio_client_clone(dialer);
	moq_tokio_client *once = moq_tokio_client_with_reconnect(dialer_copy, false);
	moq_tokio_client_free(dialer_copy);
	moq_origin_producer *received = moq_origin_spawn();
	moq_origin_consumer *remote = moq_origin_producer_consume(received);
	moq_connection *connection = moq_tokio_client_consume(once, received);
	moq_tokio_client_free(once);
	moq_origin_producer_free(received);
	CHECK(connection != NULL);

	moq_connection_established_future *establishing = moq_connection_established(connection);
	moq_connection_free(connection);
	moq_connection *established = NULL;
	AWAIT(moq_connection_established_poll(establishing, session_waker, &established, &tokio_err));
	moq_connection_established_future_free(establishing);
	CHECK(established != NULL);

	moq_origin_consumer_request_future *requesting = moq_origin_consumer_request(remote, str("live"), NULL);
	moq_broadcast_consumer *remote_broadcast = NULL;
	AWAIT(moq_origin_consumer_request_poll(requesting, session_waker, &remote_broadcast, &err));
	moq_origin_consumer_request_future_free(requesting);
	CHECK(remote_broadcast != NULL);
	moq_track_consumer *remote_track = NULL;
	CHECK(moq_broadcast_consumer_track(remote_broadcast, str("video"), &remote_track, &err));
	subscribing = moq_track_consumer_subscribe(remote_track, NULL);
	AWAIT(moq_track_consumer_subscribe_poll(subscribing, session_waker, &subscriber, &err));
	moq_track_consumer_subscribe_future_free(subscribing);
	CHECK(subscriber != NULL);
	receiving = moq_track_subscriber_recv_group(subscriber);
	group = NULL;
	AWAIT(moq_track_subscriber_recv_group_poll(receiving, session_waker, &group, &err));
	moq_track_subscriber_recv_group_future_free(receiving);
	CHECK(group != NULL);
	reading = moq_group_consumer_read_frame(group);
	frame = NULL;
	AWAIT(moq_group_consumer_read_frame_poll(reading, session_waker, &frame, &err));
	moq_group_consumer_read_frame_future_free(reading);
	CHECK(frame != NULL && is(moq_frame_frame_payload(frame), "hello"));
	moq_frame_frame_free(frame);
	moq_group_consumer_free(group);
	moq_track_subscriber_free(subscriber);
	moq_track_consumer_free(remote_track);
	moq_broadcast_consumer_free(remote_broadcast);

	moq_connection_close_future *closing = moq_connection_close(established);
	moq_connection_free(established);
	AWAIT(moq_connection_close_poll(closing, session_waker, &tokio_err));
	moq_connection_close_future_free(closing);
	moq_tokio_client_close_future *client_closing = moq_tokio_client_close(dialer);
	moq_tokio_client_free(dialer);
	AWAIT(moq_tokio_client_close_poll(client_closing, session_waker));
	moq_tokio_client_close_future_free(client_closing);
	moq_tokio_server_serve_publish_future_free(serving);
	moq_origin_consumer_free(remote);
	moq_track_producer_free(live_track);
	moq_broadcast_producer_free(live_broadcast);
	moq_origin_publication_free(live);
	moq_origin_producer_free(served);
	CHECK(tokio_err == NULL);

	if (failures == 0) {
		printf("ok\n");
	}
	return failures != 0;
}
