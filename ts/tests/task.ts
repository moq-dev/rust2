// The task fixture's harness on the translation: a deterministic FIFO executor and the
// script runner of `fixtures/task/src/exec.rs` and `harness.rs`, step for step, logging
// the same events through the translated `Log`. `differential.ts` requires the two logs
// to match byte for byte, or both runs to panic with the same message.

/** A waker, as the translation calls it. */
interface Waker extends Disposable {
	readonly data: unknown;
	readonly vtable: object;
	wake(): void;
	wakeByRef(): void;
	clone(): Waker;
}

interface Future<T> extends Disposable {
	poll(cx: Waker): T | symbol;
}

type Log = Disposable;
type Consumer = Disposable;
type Guard = Disposable;
/** A `ByteVec`, by shape: its first `length` bytes of `data`. */
export type Bytes = { data: Uint8Array; length: number };
type Channel = Disposable;

/**
 * What the harness drives: the translation's roots, and the runtime's `Pending` and
 * `u64`, whose values are `V`.
 */
export interface Fixture<V> {
	Pending: symbol;
	u64: { of(value: number): V };
	Log: {
		"new"(): Log;
		note(log: Log, event: string): void;
		text(log: Log): Bytes;
		clone$Clone(log: Log): Log;
	};
	Channel: {
		open(): Channel;
		consumer(channel: Channel): Consumer;
		send(channel: Channel, value: V): boolean;
		close(channel: Channel): boolean;
		cloneProducer(channel: Channel): boolean;
	};
	Guard: { "new"(id: number, log: Log): Guard };
	sum(rx: Consumer, guard: Guard): Future<V>;
	drain(rx: Consumer, depth: number): Future<V>;
	blocks(rx: Consumer, guard: Guard, moves: boolean): Future<V>;
	watch(rx: Consumer, guard: Guard): Future<V>;
	front(rx: Consumer, guard: Guard): Future<V>;
	drops: {
		run(log: Log, id: number, arg: number): void;
		held(rx: Consumer, guard: Guard): Future<V>;
		edited(log: Log, id: number, arg: number): void;
		dyns(log: Log, id: number, arg: number): void;
		cells(log: Log, id: number, arg: number): void;
		folded(log: Log, id: number, arg: number): void;
		rfolded(log: Log, id: number, arg: number): void;
		inside(log: Log, id: number, arg: number): void;
		compared(log: Log, id: number, arg: number): void;
		lists(log: Log, id: number, arg: number): void;
	};
	futures: {
		helpers(rx: Consumer, guard: Guard): Future<V>;
		boxed(rx: Consumer, guard: Guard): Future<V>;
		wakers(rx: Consumer, guard: Guard): Future<V>;
	};
}

/** What every waker of one task points at. */
class Wake {
	readonly task: number;
	readonly note: (event: string) => void;
	readonly queue: number[];
	queued = false;

	constructor(task: number, note: (event: string) => void, queue: number[]) {
		this.task = task;
		this.note = note;
		this.queue = queue;
	}

	schedule(): void {
		if (this.queued) return;
		this.queued = true;
		this.queue.push(this.task);
	}
}

/** The vtable of every task's waker. */
const TASK_VTABLE = {};

class TaskWaker implements Waker {
	readonly target: Wake;
	readonly vtable = TASK_VTABLE;

	constructor(target: Wake) {
		this.target = target;
	}

	/** Each clone's, as the native waker's data is the `Arc` they share. */
	get data(): unknown {
		return this.target;
	}

	wake(): void {
		this.target.note(`t${this.target.task} wake`);
		this.target.schedule();
	}

	wakeByRef(): void {
		this.wake();
	}

	clone(): Waker {
		this.target.note(`t${this.target.task} waker clone`);
		return new TaskWaker(this.target);
	}


	[Symbol.dispose](): void {
		this.target.note(`t${this.target.task} waker drop`);
	}
}

interface Task {
	future: Future<unknown>;
	waker: Waker;
	wake: Wake;
	done: boolean;
}

/** Runs tasks in the order they were woken. */
class Executor {
	readonly queue: number[] = [];
	/** Every task spawned, by id; `undefined` once dropped. */
	readonly tasks: (Task | undefined)[] = [];
	readonly note: (event: string) => void;
	readonly pending: symbol;

	constructor(note: (event: string) => void, pending: symbol) {
		this.note = note;
		this.pending = pending;
	}

	spawn(future: Future<unknown>): void {
		const task = this.tasks.length;
		this.note(`t${task} spawn`);
		const wake = new Wake(task, this.note, this.queue);
		wake.schedule();
		this.tasks.push({ future, waker: new TaskWaker(wake), wake, done: false });
	}

	run(): void {
		while (this.step());
	}

	step(): boolean {
		const task = this.queue.shift();
		if (task === undefined) return false;
		const t = this.tasks[task];
		if (t !== undefined && !t.done) this.poll(task);
		return true;
	}

	poll(task: number): void {
		const t = this.tasks[task];
		if (t === undefined) return;
		t.wake.queued = false;
		this.note(`t${task} poll`);
		const output = t.future.poll(t.waker);
		if (output === this.pending) {
			this.note(`t${task} pending`);
		} else {
			t.done = true;
			this.note(`t${task} ready ${String(output)}`);
		}
	}

	drop(task: number): void {
		const t = this.tasks[task];
		if (t === undefined) return;
		this.tasks[task] = undefined;
		this.note(`t${task} drop`);
		t.future[Symbol.dispose]();
		t.waker[Symbol.dispose]();
	}
}

/** What a step acts on: one script's fixture, log, executor, and channels. */
interface Run<V> {
	f: Fixture<V>;
	log: Log;
	note: (event: string) => void;
	exec: Executor;
	channels: Channel[];
}

/** The one of `len` channels or tasks that `arg` picks, if there are any. */
const pick = (len: number, arg: number) => (len > 0 ? arg % len : undefined);

/** A new `Guard` for the next task spawned. */
const guard = <V>(r: Run<V>) => r.f.Guard.new(r.exec.tasks.length, r.f.Log.clone$Clone(r.log));

/**
 * Each step and its name, which a script's op picks modulo their count: `harness.rs`'s
 * `steps`, in the same order.
 */
export const steps: [string, (r: Run<unknown>, arg: number) => void][] = [
	[
		"open",
		(r) => {
			r.note(`c${r.channels.length} open`);
			r.channels.push(r.f.Channel.open());
		},
	],
	[
		"sum",
		(r, arg) => {
			const c = pick(r.channels.length, arg);
			if (c !== undefined) r.exec.spawn(r.f.sum(r.f.Channel.consumer(r.channels[c]), guard(r)));
		},
	],
	[
		"drain",
		(r, arg) => {
			const c = pick(r.channels.length, arg);
			if (c !== undefined) r.exec.spawn(r.f.drain(r.f.Channel.consumer(r.channels[c]), [1, 4, 16][(arg >> 4) % 3]));
		},
	],
	[
		"send",
		(r, arg) => {
			const c = pick(r.channels.length, arg);
			if (c === undefined) return;
			r.note(`c${c} send ${arg}`);
			if (!r.f.Channel.send(r.channels[c], r.f.u64.of(arg))) r.note(`c${c} closed`);
		},
	],
	[
		"close",
		(r, arg) => {
			const c = pick(r.channels.length, arg);
			if (c === undefined) return;
			r.note(`c${c} close`);
			r.f.Channel.close(r.channels[c]);
		},
	],
	[
		"clone",
		(r, arg) => {
			const c = pick(r.channels.length, arg);
			if (c === undefined) return;
			r.note(`c${c} clone`);
			r.f.Channel.cloneProducer(r.channels[c]);
		},
	],
	["run", (r) => r.exec.run()],
	["step", (r) => r.exec.step()],
	[
		"poll",
		(r, arg) => {
			const t = pick(r.exec.tasks.length, arg);
			if (t !== undefined) r.exec.poll(t);
		},
	],
	[
		"drop",
		(r, arg) => {
			const t = pick(r.exec.tasks.length, arg);
			if (t !== undefined) r.exec.drop(t);
		},
	],
	[
		"blocks",
		(r, arg) => {
			const c = pick(r.channels.length, arg);
			if (c !== undefined) r.exec.spawn(r.f.blocks(r.f.Channel.consumer(r.channels[c]), guard(r), arg >= 128));
		},
	],
	["drops", (r, arg) => r.f.drops.run(r.log, 1000 * r.exec.tasks.length, arg)],
	[
		"held",
		(r, arg) => {
			const c = pick(r.channels.length, arg);
			if (c !== undefined) r.exec.spawn(r.f.drops.held(r.f.Channel.consumer(r.channels[c]), guard(r)));
		},
	],
	["edited", (r, arg) => r.f.drops.edited(r.log, 1000 * r.exec.tasks.length, arg)],
	[
		"helpers",
		(r, arg) => {
			const c = pick(r.channels.length, arg);
			if (c !== undefined) r.exec.spawn(r.f.futures.helpers(r.f.Channel.consumer(r.channels[c]), guard(r)));
		},
	],
	[
		"boxed",
		(r, arg) => {
			const c = pick(r.channels.length, arg);
			if (c !== undefined) r.exec.spawn(r.f.futures.boxed(r.f.Channel.consumer(r.channels[c]), guard(r)));
		},
	],
	["dyns", (r, arg) => r.f.drops.dyns(r.log, 1000 * r.exec.tasks.length, arg)],
	["cells", (r, arg) => r.f.drops.cells(r.log, 1000 * r.exec.tasks.length, arg)],
	["folded", (r, arg) => r.f.drops.folded(r.log, 1000 * r.exec.tasks.length, arg)],
	["rfolded", (r, arg) => r.f.drops.rfolded(r.log, 1000 * r.exec.tasks.length, arg)],
	[
		"watch",
		(r, arg) => {
			const c = pick(r.channels.length, arg);
			if (c !== undefined) r.exec.spawn(r.f.watch(r.f.Channel.consumer(r.channels[c]), guard(r)));
		},
	],
	[
		"front",
		(r, arg) => {
			const c = pick(r.channels.length, arg);
			if (c !== undefined) r.exec.spawn(r.f.front(r.f.Channel.consumer(r.channels[c]), guard(r)));
		},
	],
	["inside", (r, arg) => r.f.drops.inside(r.log, 1000 * r.exec.tasks.length, arg)],
	["compared", (r, arg) => r.f.drops.compared(r.log, 1000 * r.exec.tasks.length, arg)],
	[
		"wakers",
		(r, arg) => {
			const c = pick(r.channels.length, arg);
			if (c !== undefined) r.exec.spawn(r.f.futures.wakers(r.f.Channel.consumer(r.channels[c]), guard(r)));
		},
	],
	["lists", (r, arg) => r.f.drops.lists(r.log, 1000 * r.exec.tasks.length, arg)],
];

/** The script runner, as `harness.rs` runs it natively. */
export function harness<V>(f: Fixture<V>): (input: Uint8Array) => Bytes {
	return (input) => {
		const log = f.Log.new();
		const note = (event: string) => f.Log.note(log, event);
		const exec = new Executor(note, f.Pending);
		const r: Run<V> = { f, log, note, exec, channels: [] };
		for (let i = 0; i < input.length; i += 2) {
			const [name, step] = steps[input[i] % steps.length];
			note(`step ${name}`);
			step(r, input[i + 1] ?? 0);
		}
		for (let t = 0; t < exec.tasks.length; t++) exec.drop(t);
		r.channels.forEach((channel, c) => {
			note(`c${c} drop`);
			channel[Symbol.dispose]();
		});
		return f.Log.text(log);
	};
}

/** A script of steps, each by name and with its argument. */
function script(...script: [string, number][]): Uint8Array {
	return Uint8Array.from(
		script.flatMap(([name, arg]) => {
			const op = steps.findIndex(([n]) => n === name);
			if (op < 0) throw new Error(`no step ${name}`);
			return [op, arg];
		}),
	);
}

/** Scripts for the cases async must get right, run before the random ones. */
export const cases: Uint8Array[] = [
	// A future dropped before its first poll runs nothing, and drops its arguments.
	script(["open", 0], ["sum", 0], ["drain", 0], ["drop", 0], ["drop", 1]),
	// One dropped mid-await drops its awaited future, then its locals, then its arguments.
	script(["open", 0], ["sum", 0], ["run", 0], ["send", 5], ["run", 0], ["drop", 0]),
	// Polling a future after it returned `Ready` panics.
	script(["open", 0], ["sum", 0], ["run", 0], ["close", 0], ["run", 0], ["poll", 0]),
	// Every await depth, woken by sends and the channel closing.
	script(["open", 0], ["drain", 0], ["drain", 16], ["drain", 32], ["run", 0], ["send", 7], ["run", 0], ["send", 9], ["step", 0], ["close", 0], ["run", 0]),
	// The channel stays open until its last producer drops.
	script(["open", 0], ["sum", 0], ["clone", 0], ["close", 0], ["run", 0], ["send", 2], ["close", 0], ["run", 0]),
	// A task woken twice before its poll is queued once.
	script(["open", 0], ["open", 1], ["sum", 0], ["sum", 1], ["run", 0], ["send", 1], ["clone", 0], ["send", 2], ["run", 0]),
	// An async block's captures: one borrowed, one moved on one path only, and one moved
	// but only borrowed, dropped when its block returns or is cancelled mid-await.
	script(["open", 0], ["blocks", 0], ["blocks", 128], ["blocks", 0], ["run", 0], ["send", 5], ["step", 0], ["step", 0], ["drop", 2], ["run", 0]),
	// A value of 99 fails `sum`'s `assert!`.
	script(["open", 0], ["sum", 0], ["run", 0], ["send", 99], ["run", 0]),
	// Values std holds that drop, held across an await: returned, and cancelled.
	script(["open", 0], ["held", 0], ["run", 0], ["send", 5], ["run", 0]),
	script(["open", 0], ["held", 0], ["held", 1], ["run", 0], ["drop", 1], ["drop", 0]),
	// std's helpers: a `poll_fn` over a pinned `recv` and `future::pending`, returning, and
	// cancelled with the guard it owns.
	script(["open", 0], ["helpers", 0], ["run", 0], ["send", 5], ["run", 0], ["send", 77], ["run", 0]),
	script(["open", 0], ["helpers", 0], ["helpers", 1], ["run", 0], ["send", 4], ["step", 0], ["drop", 1], ["drop", 0]),
	// Boxed futures: a boxed `recv` and a boxed borrow awaited, then a queue of `dyn
	// Future`s polled through a boxed `poll_fn`, returning, and cancelled with the futures
	// the queue still holds.
	script(["open", 0], ["boxed", 0], ["run", 0], ["send", 5], ["run", 0], ["send", 6], ["step", 0], ["send", 7], ["run", 0]),
	script(["open", 0], ["boxed", 0], ["boxed", 1], ["run", 0], ["send", 4], ["run", 0], ["drop", 1], ["send", 5], ["step", 0], ["drop", 0]),
	// kio's API across crates: `Waiter::poll_future`, then a `Pending` of a `Task` polling
	// a `Poll<Poll<_>>`, whose even value waits for the next, or for the close that drops
	// it; and cancelled while it waits.
	script(["open", 0], ["watch", 0], ["run", 0], ["send", 3], ["run", 0], ["send", 4], ["run", 0], ["send", 7], ["run", 0], ["close", 0], ["run", 0]),
	script(["open", 0], ["watch", 0], ["run", 0], ["send", 1], ["run", 0], ["send", 2], ["run", 0], ["close", 0], ["run", 0]),
	script(["open", 0], ["watch", 0], ["watch", 0], ["run", 0], ["send", 6], ["step", 0], ["send", 8], ["drop", 1], ["run", 0], ["drop", 0]),
	// moq's front: a listing filled in place and reset, then values in flight that a
	// closure takes and refills each poll, read after it returns; and cancelled mid-wait.
	script(["open", 0], ["front", 0], ["run", 0], ["send", 4], ["send", 3], ["send", 7], ["send", 9], ["send", 5], ["run", 0], ["send", 6], ["run", 0], ["send", 1], ["step", 0], ["close", 0], ["run", 0]),
	script(["open", 0], ["front", 0], ["front", 0], ["run", 0], ["send", 2], ["run", 0], ["send", 8], ["step", 0], ["drop", 1], ["run", 0], ["drop", 0]),
	// Each scenario of values std holds that drop.
	...Array.from({ length: 16 }, (_, arg) => script(["drops", arg])),
	// Each of a `Vec`'s edits, its, a tuple's, and a box's `clone`, and `Ord`'s defaults.
	...Array.from({ length: 9 }, (_, arg) => script(["edited", arg])),
	// What boxed and shared trait objects drop, `dyn Any`s, and `dyn Fn*`s.
	...Array.from({ length: 4 }, (_, arg) => script(["dyns", arg])),
	// What one-time cells drop.
	...Array.from({ length: 6 }, (_, arg) => script(["cells", arg])),
	// Each scenario of adaptors' closures that drop, folded either way.
	...Array.from({ length: 42 }, (_, arg) => script(["folded", arg])),
	...Array.from({ length: 9 }, (_, arg) => script(["rfolded", arg])),
	// Where std drops values inside its iterators and formatting.
	...[...Array.from({ length: 10 }, (_, arg) => arg), 19, 29].map((arg) => script(["inside", arg])),
	// What std's comparisons of iterators drop.
	...Array.from({ length: 9 }, (_, arg) => script(["compared", arg])),
	// What a `LinkedList` drops, and when.
	...Array.from({ length: 8 }, (_, arg) => script(["lists", arg])),
	// Wakers' identities, as kio's `Polling` keeps one: a task's own across its polls, then
	// another task's in between.
	script(["open", 0], ["wakers", 0], ["run", 0], ["send", 3], ["run", 0], ["close", 0], ["run", 0]),
	script(["open", 0], ["wakers", 0], ["wakers", 0], ["run", 0], ["send", 2], ["step", 0], ["step", 0], ["drop", 1], ["send", 5], ["run", 0], ["drop", 0]),
];
