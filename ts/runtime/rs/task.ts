// Ownership and async, for rust2-ts output: what Rust's `Drop`, `Arc`, `Mutex`, atomics,
// one-time cells, and futures do that plain JS does not. `rs.ts` re-exports all of it.
//
// A value whose drop JS can observe has `[Symbol.dispose]`, which runs its `Drop` impl
// and drops its fields. `None` is `undefined`, which drops as nothing.

import { Abort, arrayAssign, type ByteSlice, type CloneFrom, cloneOver, type Debug, type DebugStruct, debugStructField, debugStructFinish, debugStructFinishNonExhaustive, debugTupleField, debugTupleFinish, Err, type FmtResult, type Formatter, fmtBool, fmtDebugStruct, fmtDebugTuple, fmtDebugTupleFields, fmtPad, fmtStr, type None, NULL, objectCopy, OK, Ok, Panic, panic, type Place, place, ptrErase, ptrHome, ptrNowhere, ptrWrite, type Result, Slice, stringAsBytes, type Vec } from "../rs.ts";
import type { Hasher } from "./hash.ts";
import type { Location } from "./location.ts";

// Drops

/** Drop `value`: run its `[Symbol.dispose]`, if it has one. */
export function drop(value: unknown): void {
	(value as Partial<Disposable> | null | undefined)?.[Symbol.dispose]?.();
}

/** Drop every element of `values`, an array or a boxed slice, from `from` on, in order, by `f` if it has no `[Symbol.dispose]`. */
export function dropAll<T>(values: ArrayLike<T> | Slice<T> | undefined, f: (value: T) => void = drop, from = 0): void {
	if (values instanceof Slice) dropRange(values.data, f, values.start + from, values.start + values.length);
	else if (values !== undefined) dropRange(values, f, from, values.length);
}

/** Drop every item of `v` in order, as `dropAll` does. */
export function dropVec<T>(v: Vec<T> | undefined, f: (value: T) => void = drop): void {
	if (v !== undefined) dropRange(v.items, f, 0, v.items.length);
}

/** Drop `values[from..to]` as `dropAll` does. */
export function dropRange<T>(values: ArrayLike<T>, f: (value: T) => void, from: number, to: number): void {
	for (; from < to; from++) f(values[from]);
}

// Closures. One that owns a value that drops is an arrow function with
// `[Symbol.dispose]`, which drops what it owns. Calling one that `FnOnce` calls consumes
// it, as its body drops what it owns; one that `Fn` or `FnMut` call drops after a call
// through `FnOnce`, as rustc's shim does. One whose clone can't be itself is made by a
// function of its captures, which makes the clone again of theirs.

/** A closure `f`, which owns what `dispose` drops. */
export function closure<F extends (...args: never[]) => unknown>(f: F, dispose: () => void): F & Disposable {
	return Object.assign(f, { [Symbol.dispose]: dispose });
}

/** What makes a closure's clone, of one that a clone can't share. */
const CLONE: unique symbol = /* @__PURE__ */ Symbol("clone");

/**
 * A closure `f` whose `Clone` is `clone`: one that writes what it captured, or captured
 * what clones other than as itself, made again of its captures' clones.
 */
export function cloning<F extends (...args: never[]) => unknown>(f: F, clone: () => F): F {
	return Object.assign(f, { [CLONE]: clone });
}

/** `Clone::clone` of a closure that `cloning` made. */
export function closureClone<F>(f: F): F {
	return (f as F & { [CLONE]: () => F })[CLONE]();
}

/** What makes the future of a call that consumes an async closure. */
const BY_MOVE: unique symbol = /* @__PURE__ */ Symbol("byMove");

/**
 * An async closure `f` that `Fn` or `FnMut` call and that owns what drops: a call that
 * consumes it calls `once`, whose future owns what `f`'s would borrow.
 */
export function byMove<F extends (...args: never[]) => unknown>(f: F, once: F): F {
	return Object.assign(f, { [BY_MOVE]: once });
}

/**
 * `FnOnce::call_once` of a closure that `Fn` or `FnMut` call: `f(...args)`, then `f`
 * drops; or, of an async closure `byMove` made, the future that owns what it captured.
 */
export function callOnce<A extends unknown[], R>(f: (...args: A) => R, ...args: A): R {
	const once = (f as { [BY_MOVE]?: (...args: A) => R })[BY_MOVE];
	if (once !== undefined) return once(...args);
	const r = f(...args);
	drop(f);
	return r;
}

/**
 * `call(f)`, then `f` dropped by `by`: a std function that calls `f`, or reads a range `f`,
 * and keeps nothing of it drops it as it returns.
 */
export function dropAfter<T, R>(f: T, call: (f: T) => R, by: (f: T) => void = drop): R {
	const r = call(f);
	by(f);
	return r;
}

/** A closure that `Fn` or `FnMut` call, as one `FnOnce` calls: a call drops it after. */
export function once<A extends unknown[], R>(f: (...args: A) => R): ((...args: A) => R) & Disposable {
	return closure((...args: A) => callOnce(f, ...args), () => drop(f));
}

/**
 * The function `make` makes of `f`, which calls it with what std lends it as translated code
 * reads it: its clone is made of `f`'s clone, and it drops `f`.
 */
export function lending<F, G extends (...args: never[]) => unknown>(f: F, make: (f: F) => G): G & Disposable {
	return closure(
		cloning(make(f), () => lending(closureClone(f), make)),
		() => drop(f),
	);
}

/** A function that calls `f` but never drops it, for a reference to a closure that something would drop. */
export function lent<A extends unknown[], R>(f: (...args: A) => R): (...args: A) => R {
	return (...args: A) => f(...args);
}

// Shared ownership

/** `Arc<T>` and `Rc<T>`: every clone is this one object, counted, as are its `Weak`s. */
export class Arc<T> implements Disposable {
	strong = 1;
	weak = 0;
	value: T;

	constructor(value: T) {
		this.value = value;
	}

	[Symbol.dispose](): void {
		if (--this.strong === 0) drop(this.value);
	}
}

/**
 * `Arc<dyn Trait>` unsized from `arc`, which it shares one count with, as Rust's one
 * allocation does: `value` is the trait object over `arc`'s value, which the last of
 * either drops.
 */
export class ArcDyn<T, U> implements Disposable {
	readonly arc: Arc<T>;
	readonly value: U;

	constructor(arc: Arc<T>, value: U) {
		this.arc = arc;
		this.value = value;
	}

	get strong(): number {
		return this.arc.strong;
	}

	set strong(strong: number) {
		this.arc.strong = strong;
	}

	get weak(): number {
		return this.arc.weak;
	}

	set weak(weak: number) {
		this.arc.weak = weak;
	}

	[Symbol.dispose](): void {
		if (--this.arc.strong === 0) drop(this.value);
	}
}

/** `Arc::ptr_eq` of trait objects, which compares the `Arc`s they were made from. */
export function arcPtrEq<T>(l: Arc<T>, r: Arc<T>): boolean {
	return arcRoot(l) === arcRoot(r);
}

function arcRoot(arc: Arc<unknown>): Arc<unknown> {
	while (arc instanceof ArcDyn) arc = arc.arc;
	return arc;
}

/** The `Arc<T>` each `Arc::downcast` of an `Arc<dyn Any>` made from a box gives, which is one allocation. */
const downcasts = new WeakMap<Arc<unknown>, Arc<unknown>>();

/**
 * `Arc::downcast` of an `Arc<dyn Any>`, if its object is a `class`: the `Arc` it was made
 * from, or one of what the object holds, by `held`, sharing the count of one made from a box.
 */
export function arcDowncast<T, U, C>(
	arc: Arc<U>,
	class_: abstract new (...args: never[]) => C,
	held: (object: C) => T,
): Result<Arc<T>, Arc<U>> {
	if (!(arc.value instanceof class_)) return Err(arc);
	if (arc instanceof ArcDyn) return Ok(arc.arc as Arc<T>);
	let typed = downcasts.get(arc) as Arc<T> | undefined;
	if (typed === undefined) {
		typed = new ArcDyn(arc, held(arc.value));
		downcasts.set(arc, typed);
	}
	return Ok(typed);
}

/**
 * The `Arc<[u8]>` each `From` of an `Arc<str>` gives, which is one allocation, and the text
 * it was made of. Other text there was written through a unique `Arc<str>`, so no `Arc<[u8]>`
 * of the old text is left.
 */
const strBytes = new WeakMap<Arc<string>, { text: string; bytes: Arc<ByteSlice> }>();

/** `Arc<[u8]>::from` an `Arc<str>`: its bytes, sharing its count as std shares the allocation. */
export function arcStrBytes(arc: Arc<string>): Arc<ByteSlice> {
	let made = strBytes.get(arc);
	if (made?.text !== arc.value) {
		made = { text: arc.value, bytes: new ArcDyn(arc, stringAsBytes(arc.value)) };
		strBytes.set(arc, made);
	}
	return made.bytes;
}

/**
 * Drop `arc`, whose value has no `[Symbol.dispose]` that drops it: the last one drops it
 * by `f`, or not at all without one, as a borrow.
 */
export function arcDrop<T>(arc: Arc<T>, f?: (value: T) => void): void {
	if (--arc.strong === 0) f?.(arc.value);
}

/** `Arc::clone`: the same value, counted once more. */
export function arcClone<T>(arc: Arc<T>): Arc<T> {
	arc.strong++;
	return arc;
}

/**
 * `Arc::into_inner`: the value of the last strong `Arc`, after which its `Weak`s no
 * longer upgrade, or else `none`, with `arc` no longer counted.
 */
export function arcIntoInner<T>(arc: Arc<T>): T | undefined;
export function arcIntoInner<T>(arc: Arc<T>, none: None): T | None;
export function arcIntoInner<T>(arc: Arc<T>, none?: None): T | None | undefined {
	return --arc.strong === 0 ? arc.value : none;
}

/** `Arc::get_mut`: `arc`'s value, if no other `Arc` or `Weak` shares it. */
export function arcGetMut<T>(arc: Arc<T>): T | undefined {
	return arcUnique(arc) ? arc.value : undefined;
}

/** `Arc::get_mut` of a value JS copies: where it is, if no other `Arc` or `Weak` shares it. */
export function arcGetMutPlace<T>(arc: Arc<T>): Place<T> | undefined {
	return arcUnique(arc) ? valuePlace(arc) : undefined;
}

/**
 * `Arc::make_mut`: the value of the `Arc` at `p`, made its own. Another `Arc` sharing it
 * leaves a clone by `clone`, or the value itself if none, in a new `Arc` at `p`, and drops
 * the old one by `drop`, which is the last if the clone released the others. Only `Weak`s
 * sharing it leave them dangling as the value moves to a new `Arc`, copied by `moved`
 * where a pointer to it could tell the two allocations apart.
 */
export function arcMakeMut<T>(
	p: Place<Arc<T>>,
	clone: ((value: T) => T) | undefined,
	drop: (arc: Arc<T>) => void,
	moved?: (value: T) => T,
): T {
	return arcOwn(p, clone, drop, moved).value;
}

/** `Arc::make_mut` of a value JS copies: where it is. */
export function arcMakeMutPlace<T>(
	p: Place<Arc<T>>,
	clone: ((value: T) => T) | undefined,
	drop: (arc: Arc<T>) => void,
	moved?: (value: T) => T,
): Place<T> {
	return valuePlace(arcOwn(p, clone, drop, moved));
}

function arcUnique(arc: Arc<unknown>): boolean {
	return arc.strong === 1 && arc.weak === 0;
}

function arcOwn<T>(
	p: Place<Arc<T>>,
	clone: ((value: T) => T) | undefined,
	drop: (arc: Arc<T>) => void,
	moved?: (value: T) => T,
): Arc<T> {
	const arc = p.v;
	if (arc.strong !== 1) {
		p.v = new Arc(clone ? clone(arc.value) : arc.value);
		drop(arc);
	} else if (arc.weak !== 0) {
		arc.strong = 0;
		p.v = new Arc(moved ? moved(arc.value) : arc.value);
	}
	return p.v;
}

/** Where the value of an `Arc`, a `Mutex`, or a `RefCell` is, which a `&mut` to it lends where JS copies it. */
export function valuePlace<T>(holder: { value: T }): Place<T> {
	return place(
		() => holder.value,
		(value) => {
			holder.value = value;
		},
	);
}

/** The place of each `Arc`'s value that `arcPlace` made. */
const places = new WeakMap<Arc<unknown>, Place<unknown>>();

/**
 * `Arc::as_ptr` of a value JS copies: the place of its value, one for the allocation the
 * `Arc` is, as its address is.
 */
export function arcPlace<T>(arc: Arc<T>): Place<T> {
	let pos = places.get(arc);
	if (pos === undefined) places.set(arc, (pos = valuePlace(arc)));
	return pos as Place<T>;
}

/** The `Arc` each raw pointer `Arc::into_raw` or `as_ptr` gave points into, by that pointer, which is its identity. */
const RAW_ARCS = /* @__PURE__ */ new WeakMap<object, Arc<unknown>>();

/** The `Rc` each raw pointer `Rc::into_raw` or `as_ptr` gave points into, as `RAW_ARCS`. */
const RAW_RCS = /* @__PURE__ */ new WeakMap<object, Arc<unknown>>();

/**
 * `Arc::into_raw` or `as_ptr` of `arc`, an `Arc` or an `Rc` as `of` says: `p`, the pointer
 * to its value, which `arcFromRaw` takes back to `arc`.
 */
export function arcRaw<P extends object>(arc: Arc<unknown>, p: P, of: "Arc" | "Rc"): P {
	(of === "Arc" ? RAW_ARCS : RAW_RCS).set(p, arc);
	return p;
}

/**
 * `Arc::from_raw`, of an `Arc` or an `Rc` as `of` says: the one `p` points into, which
 * `into_raw` or `as_ptr` gave, or arithmetic moved back to, and a count of which it takes,
 * as std's does. Any other pointer, or one to a value already dropped, fails, as JS keeps
 * no count beside it.
 */
export function arcFromRaw<T>(p: object, of: "Arc" | "Rc"): Arc<T> {
	const arc = (of === "Arc" ? RAW_ARCS : RAW_RCS).get(ptrHome(p));
	if (arc === undefined) throw new Error(`rust2-ts: \`${of}::from_raw\` of a pointer no \`${of}::into_raw\` or \`as_ptr\` gave`);
	if (arc.strong === 0) throw new Error(`rust2-ts: \`${of}::from_raw\` of a pointer to a value already dropped`);
	return arc as Arc<T>;
}

/**
 * `p as *const ()`, as `ptrErase` casts it, of a pointer to a value JS copies: the place
 * of an `Arc`'s or `Rc`'s value that `into_raw` or `as_ptr` gave, or arithmetic moved back
 * to, which is its one place as its address is, or one at no memory. Any other place is a
 * borrow's own, which no other pointer to the value would compare as, so it fails.
 */
export function arcErase(p: object, type: string, align: number): Place<undefined> {
	const home = ptrHome(p);
	if (!RAW_ARCS.has(home) && !RAW_RCS.has(home) && !ptrNowhere(home)) {
		throw new Error(`rust2-ts: a pointer to \`${type}\` cast to one to \`()\`, which only an \`Arc\`'s or \`Rc\`'s raw pointer keeps the place of`);
	}
	return ptrErase(home, type, align);
}

/** What a `Weak` that never upgrades points at, as `Weak::new`'s does: no strong count. */
const DANGLING: Arc<never> = /* @__PURE__ */ dangling();

function dangling<T>(): Arc<T> {
	const arc = new Arc<T>(undefined as T);
	arc.strong = 0;
	return arc;
}

/**
 * `Arc::new_cyclic`: the value `f` makes from a `Weak` of it, which upgrades only once `f`
 * returns. That `Weak` then counts no more, as the one every strong `Arc` shares.
 */
export function arcNewCyclic<T>(f: (weak: Weak<T>) => T): Arc<T> {
	const arc = dangling<T>();
	arc.value = f(new Weak(arc));
	arc.strong = 1;
	arc.weak--;
	return arc;
}

/**
 * `Weak<T>`, of `arc`, or else dangling, counted in `arc` until it drops. One unsized to a
 * `Weak<dyn Trait>` keeps the `Arc` it was made from, which `view` makes an `Arc<dyn Trait>`
 * of as it upgrades, since `Arc::new_cyclic`'s value may not be there yet.
 */
export class Weak<T> implements Disposable {
	readonly arc: Arc<unknown>;
	readonly view: ((arc: Arc<never>) => Arc<T>) | undefined;

	constructor(arc: Arc<T> = DANGLING, view?: (arc: Arc<never>) => Arc<T>) {
		this.arc = arc;
		this.view = view;
		arc.weak++;
	}

	[Symbol.dispose](): void {
		this.arc.weak--;
	}
}

/** `weak`, a `Weak<T>`, unsized to a `Weak<dyn Trait>` of what `view` makes of its `Arc`. */
export function weakDyn<T, U>(weak: Weak<T>, view: (arc: Arc<T>) => Arc<U>): Weak<U> {
	const dyn = new Weak(weak.arc as Arc<U>, view as (arc: Arc<never>) => Arc<U>);
	weak.arc.weak--;
	return dyn;
}

/**
 * `value`, a struct whose last field unsizes to a trait object, as the struct of that
 * object: its other fields, a cell's borrows, and a mutex's lock are `value`'s own, the
 * last field is the object `field`'s one function makes of `value` wherever it is read,
 * and its drop is `value`'s, by `by`.
 */
export function tailDyn<T extends object, F extends { readonly [P in keyof T]?: (value: T) => unknown }>(
	value: T,
	field: F,
	by?: (value: T) => void,
): Omit<T, keyof F> & { [P in keyof F]: ReturnType<NonNullable<F[P]>> } & Disposable {
	const key = Object.keys(field)[0] as keyof F;
	const object = field[key] as (value: T) => unknown;
	return new Proxy(value, new Tail(key, object, () => by?.(value))) as never;
}

/** Whether two `tailDyn` views, or views of them, are of one struct. */
export function tailSame(x: object, y: object): boolean {
	return tailed(x) === tailed(y);
}

/** What a `tailDyn` view reads as the struct it is of. */
const TAILED: unique symbol = /* @__PURE__ */ Symbol("tailed");

function tailed(view: object): object {
	for (let of: object | undefined; (of = (view as { [TAILED]?: object })[TAILED]) !== undefined; ) view = of;
	return view;
}

/** What `tailDyn` makes: a view whose last field is made as it is read, and whose drop is `dispose`. */
class Tail<T extends object, U> implements ProxyHandler<T> {
	private readonly key: PropertyKey;
	private readonly object: (value: T) => U;
	private readonly dispose: () => void;

	constructor(key: PropertyKey, object: (value: T) => U, dispose: () => void) {
		this.key = key;
		this.object = object;
		this.dispose = dispose;
	}

	get(target: T, key: PropertyKey): unknown {
		if (key === this.key) return this.object(target);
		if (key === Symbol.dispose) return this.dispose;
		if (key === TAILED) return target;
		return Reflect.get(target, key);
	}
}

/** `Weak::upgrade`: a new `Arc`, or `undefined` once none is left. */
export function weakUpgrade<T>(weak: Weak<T>): Arc<T> | undefined {
	if (weak.arc.strong === 0) return undefined;
	const arc = arcClone(weak.arc as Arc<never>);
	return weak.view === undefined ? arc : weak.view(arc);
}

/** `Weak::clone`. */
export function weakClone<T>(weak: Weak<T>): Weak<T> {
	return new Weak(weak.arc as Arc<T>, weak.view);
}

/** `Weak::weak_count`: its `Arc`'s, or 0 once no strong one is left, as for a dangling one. */
export function weakWeakCount<T>(weak: Weak<T>): number {
	return weak.arc.strong > 0 ? weak.arc.weak : 0;
}

/**
 * `Mutex<T>`, which a single thread never contends, and its guard: a lock is the mutex
 * itself, `locked` until the guard drops. A panic aborts, so nothing poisons it.
 */
export class Mutex<T> implements Disposable {
	value: T;
	locked = false;

	constructor(value: T) {
		this.value = value;
	}

	[Symbol.dispose](): void {
		drop(this.value);
	}
}

/** `Debug` of a `Mutex`: its value, locked while it is written, or `"<locked>"`, a string, while a guard holds it. */
export function mutexDebug<T>(mutex: Mutex<T>, f: Formatter, debug: Debug<T>): FmtResult {
	const b = fmtDebugStruct(f, "Mutex");
	if (mutex.locked) debugStructField(b, "data", (f) => fmtStr("<locked>", f));
	else {
		mutexLock(mutex);
		debugStructField(b, "data", (f) => debug(mutex.value, f));
		mutexUnlock(mutex);
	}
	return lockFinish(b);
}

/** A lock's `Debug` after its `data`: that it is never poisoned, as a panic aborts. */
function lockFinish(b: DebugStruct): FmtResult {
	return debugStructFinishNonExhaustive(debugStructField(b, "poisoned", (f) => fmtBool(false, f)));
}

/** `TryLockError<T>`, tagged as the crate's own enums are; a panic aborts, so it is never `Poisoned`. */
export type TryLockError<T> = { readonly tag: "Poisoned"; readonly _0: T } | { readonly tag: "WouldBlock" };

const WOULD_BLOCK: Result<never, TryLockError<never>> = /* @__PURE__ */ Err({ tag: "WouldBlock" });

/** `Mutex::lock`: its guard, which is the mutex. Locking it again while held panics, as wasm32's does. */
export function mutexLock<T>(mutex: Mutex<T>): Mutex<T> {
	if (mutex.locked) panic("assertion `left == right` failed: cannot recursively acquire mutex\n  left: true\n right: false", "mutex_lock", true);
	mutex.locked = true;
	return mutex;
}

/** `Mutex::try_lock`: its guard, or `WouldBlock` while another guard holds it. */
export function mutexTryLock<T>(mutex: Mutex<T>): Result<Mutex<T>, TryLockError<Mutex<T>>> {
	if (mutex.locked) return WOULD_BLOCK;
	mutex.locked = true;
	return Ok(mutex);
}

/**
 * What a guard's map lends: where its part of the value is, which it reads and writes as
 * `value`, as its cell or mutex does its own value.
 */
abstract class Mapped<T> implements Disposable {
	readonly at: Place<T>;

	constructor(pos: Place<T>) {
		this.at = pos;
	}

	get value(): T {
		return this.at.v;
	}

	set value(value: T) {
		this.at.v = value;
	}

	/** A guard drops nothing of what it lends. */
	[Symbol.dispose](): void {}
}

/** A `MappedMutexGuard`: a `Mapped` of a part of the value of `mutex`, a guard it took over. */
export class MutexMap<T> extends Mapped<T> implements Mutex<T> {
	readonly mutex: Mutex<unknown>;

	constructor(pos: Place<T>, mutex: Mutex<unknown>) {
		super(pos);
		this.mutex = mutex;
	}

	/** Its mutex's lock, which its drop releases. */
	get locked(): boolean {
		return this.mutex.locked;
	}

	set locked(locked: boolean) {
		this.mutex.locked = locked;
	}
}

/** A `MutexGuard`'s drop, or a `MappedMutexGuard`'s. */
export function mutexUnlock<T>(guard: Mutex<T>): void {
	guard.locked = false;
}

/** `MutexGuard::map`: a `MappedMutexGuard` of the part of `guard`'s value at `at`, which takes over its lock. */
export function mutexMap<U>(guard: Mutex<unknown>, pos: Place<U>): Mutex<U> {
	return new MutexMap(pos, guard);
}

/** `MutexGuard::filter_map`: `mutexMap` of the part at `at`, or `guard` back if there is none. */
export function mutexFilterMap<T, U>(guard: Mutex<T>, pos: Place<U> | undefined): Result<Mutex<U>, Mutex<T>> {
	return pos === undefined ? Err(guard) : Ok(new MutexMap(pos, guard));
}

/**
 * `RefCell<T>` and `RwLock<T>`, which one thread can't tell apart but by how a borrow
 * that conflicts fails, and their guards: a borrow is the cell itself, `borrows` counting
 * the shared ones held, or the negated count of exclusive ones, which only a split makes
 * more than one of.
 */
export class RefCell<T> implements Disposable {
	value: T;
	borrows = 0;

	constructor(value: T) {
		this.value = value;
	}

	[Symbol.dispose](): void {
		drop(this.value);
	}
}

/** `RefCell::borrow`: a shared borrow, which panics while an exclusive one is held. */
export function refCellBorrow<T>(cell: RefCell<T>, at?: Location): RefCell<T> {
	if (cell.borrows < 0) panic("RefCell already mutably borrowed", at, true);
	cell.borrows++;
	return cell;
}

/** `RefCell::borrow_mut`: an exclusive borrow, which panics while any other is held. */
export function refCellBorrowMut<T>(cell: RefCell<T>, at?: Location): RefCell<T> {
	if (cell.borrows !== 0) panic("RefCell already borrowed", at, true);
	cell.borrows = -1;
	return cell;
}

/** `RefCell::replace`: its value, which `value` replaces, as an exclusive borrow would. */
export function refCellReplace<T>(cell: RefCell<T>, value: T, at?: Location): T {
	if (cell.borrows !== 0) panic("RefCell already borrowed", at, true);
	const old = cell.value;
	cell.value = value;
	return old;
}

/**
 * `RefCell::replace_with`: its value, which what `f` returns of it, or of its place if
 * `copied`, where JS copies it, replaces, exclusively borrowed until then.
 */
export function refCellReplaceWith<T>(cell: RefCell<T>, f: (value: never) => T, copied = false, at?: Location): T {
	refCellBorrowMut(cell, at);
	const pos = copied
		? place(
				() => cell.value,
				(v) => {
					cell.value = v;
				},
			)
		: cell.value;
	const replacement = f(pos as never);
	const old = cell.value;
	cell.value = replacement;
	refMutRelease(cell);
	return old;
}

/** `Debug` of a `RefCell`: its value, borrowed while it is written, or `<borrowed>` unpadded while borrowed exclusively. */
export function refCellDebug<T>(cell: RefCell<T>, f: Formatter, debug: Debug<T>, at?: Location): FmtResult {
	const b = fmtDebugStruct(f, "RefCell");
	if (cell.borrows < 0) debugStructField(b, "value", (f) => f.out.writeStr("<borrowed>"));
	else {
		refCellBorrow(cell, at);
		debugStructField(b, "value", (f) => debug(cell.value, f));
		refRelease(cell);
	}
	return debugStructFinish(b);
}

/** `Debug` of an `RwLock`: its value, read while it is written, or `<locked>` unpadded while written. */
export function rwLockDebug<T>(lock: RefCell<T>, f: Formatter, debug: Debug<T>): FmtResult {
	const b = fmtDebugStruct(f, "RwLock");
	if (lock.borrows < 0) debugStructField(b, "data", (f) => f.out.writeStr("<locked>"));
	else {
		rwLockRead(lock);
		debugStructField(b, "data", (f) => debug(lock.value, f));
		refRelease(lock);
	}
	return lockFinish(b);
}

/** `RwLock::read`: a shared guard, which aborts while a write is held, as wasm32's does. */
export function rwLockRead<T>(lock: RefCell<T>): RefCell<T> {
	if (lock.borrows < 0) throw new Abort("rwlock locked for writing");
	lock.borrows++;
	return lock;
}

/** `RwLock::write`: an exclusive guard, which aborts while any other is held, as wasm32's does. */
export function rwLockWrite<T>(lock: RefCell<T>): RefCell<T> {
	if (lock.borrows !== 0) throw new Abort("rwlock locked for reading");
	lock.borrows = -1;
	return lock;
}

/** `RwLock::try_read`: a shared guard, or `WouldBlock` while a write is held. */
export function rwLockTryRead<T>(lock: RefCell<T>): Result<RefCell<T>, TryLockError<RefCell<T>>> {
	if (lock.borrows < 0) return WOULD_BLOCK;
	lock.borrows++;
	return Ok(lock);
}

/** `RwLock::try_write`: an exclusive guard, or `WouldBlock` while any other is held. */
export function rwLockTryWrite<T>(lock: RefCell<T>): Result<RefCell<T>, TryLockError<RefCell<T>>> {
	if (lock.borrows !== 0) return WOULD_BLOCK;
	lock.borrows = -1;
	return Ok(lock);
}

/** `RwLockWriteGuard::downgrade`: `guard`, a lock's own, still holding it, now shared. */
export function rwLockDowngrade<T>(guard: RefCell<T>): RefCell<T> {
	guard.borrows = 1;
	return guard;
}

/** A guard a map of a cell's guard made: a `Mapped` of a part of its value, whose borrow that cell counts. */
export class RefMap<T> extends Mapped<T> implements RefCell<T> {
	/** A cell's count, which a guard's cell keeps instead. */
	borrows = 0;
	readonly cell: RefCell<unknown>;

	constructor(pos: Place<T>, cell: RefCell<unknown>) {
		super(pos);
		this.cell = cell;
	}
}

/** The cell whose borrow `r`, a guard, holds. */
function refCellOf(r: RefCell<unknown>): RefCell<unknown> {
	return r instanceof RefMap ? r.cell : r;
}

/** A `Ref`'s or an `RwLockReadGuard`'s drop. */
export function refRelease<T>(r: RefCell<T>): void {
	refCellOf(r).borrows--;
}

/** `Ref::map`: a `Ref` of `part` of `r`'s value, which takes over `r`'s borrow. */
export function refMap<U>(r: RefCell<unknown>, part: U): RefCell<U> {
	return new RefMap({ v: part }, refCellOf(r));
}

/** `Ref::filter_map`: `refMap` of `part`, or `r` back if there is none. */
export function refFilterMap<T, U>(r: RefCell<T>, part: U | undefined): Result<RefCell<U>, RefCell<T>> {
	return part === undefined ? Err(r) : Ok(new RefMap({ v: part }, refCellOf(r)));
}

/** `Ref::try_map`: `refMap` of the part `f` gave, or `r` back beside its error. */
export function refTryMap<T, U, E>(r: RefCell<T>, part: Result<U, E>): Result<RefCell<U>, [RefCell<T>, E]> {
	return part.ok ? Ok(new RefMap({ v: part.value }, refCellOf(r))) : Err([r, part.error]);
}

/** `Ref::map_split`: `Ref`s of the two `parts` of `r`'s value, which share `r`'s borrow. */
export function refMapSplit<U, V>(r: RefCell<unknown>, [u, v]: [U, V]): [RefCell<U>, RefCell<V>] {
	const cell = refCellOf(r);
	cell.borrows++;
	return [new RefMap({ v: u }, cell), new RefMap({ v }, cell)];
}

/** `Ref::clone`: `r` again, a shared borrow of its own, which never conflicts. */
export function refClone<T>(r: RefCell<T>): RefCell<T> {
	refCellOf(r).borrows++;
	return r;
}

/**
 * Where `part`, an object a `&mut` lends, is: the object itself, which a whole value is
 * written into in place, as a write through that `&mut` would.
 */
export function partPlace<T extends object>(part: T): Place<T> {
	return place(
		() => part,
		(value) => {
			if (Array.isArray(part)) arrayAssign(part, value as unknown[]);
			else ptrWrite(part, value);
		},
	);
}

/** `RefMut::map`: a `RefMut` of the part of `r`'s value at `at`, which takes over `r`'s borrow. */
export function refMutMap<U>(r: RefCell<unknown>, pos: Place<U>): RefCell<U> {
	return new RefMap(pos, refCellOf(r));
}

/** `RefMut::filter_map`: `refMutMap` of the part at `at`, or `r` back if there is none. */
export function refMutFilterMap<T, U>(r: RefCell<T>, pos: Place<U> | undefined): Result<RefCell<U>, RefCell<T>> {
	return pos === undefined ? Err(r) : Ok(new RefMap(pos, refCellOf(r)));
}

/** `RefMut::try_map`: `refMutMap` of the part at where `f` gave, or `r` back beside its error. */
export function refMutTryMap<T, U, E>(r: RefCell<T>, pos: Result<Place<U>, E>): Result<RefCell<U>, [RefCell<T>, E]> {
	return pos.ok ? Ok(new RefMap(pos.value, refCellOf(r))) : Err([r, pos.error]);
}

/** `RefMut::map_split`: `RefMut`s of the parts of `r`'s value at `a` and `b`, which share `r`'s borrow. */
export function refMutMapSplit<U, V>(r: RefCell<unknown>, [a, b]: [Place<U>, Place<V>]): [RefCell<U>, RefCell<V>] {
	const cell = refCellOf(r);
	cell.borrows--;
	return [new RefMap(a, cell), new RefMap(b, cell)];
}

/** `RefCell::try_borrow`'s error: the cell is mutably borrowed. */
export class BorrowError {}

/** `RefCell::try_borrow_mut`'s error: the cell is borrowed. */
export class BorrowMutError {}

const BORROW_ERROR: Result<never, BorrowError> = /* @__PURE__ */ Err(/* @__PURE__ */ new BorrowError());
const BORROW_MUT_ERROR: Result<never, BorrowMutError> = /* @__PURE__ */ Err(/* @__PURE__ */ new BorrowMutError());

/** `RefCell::try_borrow`: a shared borrow, or `BorrowError` while an exclusive one is held. */
export function refCellTryBorrow<T>(cell: RefCell<T>): Result<RefCell<T>, BorrowError> {
	if (cell.borrows < 0) return BORROW_ERROR;
	cell.borrows++;
	return Ok(cell);
}

/** `RefCell::try_borrow_mut`: an exclusive borrow, or `BorrowMutError` while any other is held. */
export function refCellTryBorrowMut<T>(cell: RefCell<T>): Result<RefCell<T>, BorrowMutError> {
	if (cell.borrows !== 0) return BORROW_MUT_ERROR;
	cell.borrows = -1;
	return Ok(cell);
}

/** `RefCell::undo_leak`: `cell`, borrowed no longer, as the guards it leaked are forgotten. */
export function refCellUndoLeak<T>(cell: RefCell<T>): RefCell<T> {
	cell.borrows = 0;
	return cell;
}

/** `Debug` of `BorrowError`. */
export function borrowErrorDebug(_: BorrowError, f: Formatter): FmtResult {
	return f.out.writeStr("BorrowError");
}

/** `Display` of `BorrowError`. */
export function borrowErrorDisplay(_: BorrowError, f: Formatter): FmtResult {
	return fmtPad(f, "RefCell already mutably borrowed");
}

/** `Debug` of `BorrowMutError`. */
export function borrowMutErrorDebug(_: BorrowMutError, f: Formatter): FmtResult {
	return f.out.writeStr("BorrowMutError");
}

/** `Display` of `BorrowMutError`. */
export function borrowMutErrorDisplay(_: BorrowMutError, f: Formatter): FmtResult {
	return fmtPad(f, "RefCell already borrowed");
}

/**
 * `Clone` of a `RefCell`: one of its value's clone by `clone`, or the value itself if
 * that is undefined, made while borrowing it, which panics while it is mutably borrowed.
 */
export function refCellClone<T>(cell: RefCell<T>, clone?: (value: T) => T, at?: Location): RefCell<T> {
	refCellBorrow(cell, at);
	const value = clone ? clone(cell.value) : cell.value;
	refRelease(cell);
	return new RefCell(value);
}

/**
 * `clone_from` of a `RefCell`: `src`'s value cloned over its own, as `cloneOver` clones
 * it, while borrowing `src`, which panics while it is mutably borrowed.
 */
export function refCellCloneFrom<T>(cell: RefCell<T>, src: RefCell<T>, clone?: (value: T) => T, drop?: (item: T) => void, over?: CloneFrom<T>, at?: Location): RefCell<T> {
	refCellBorrow(src, at);
	cell.value = cloneOver(cell.value, src.value, clone, drop, over);
	refRelease(src);
	return cell;
}

/** `refCellCloneFrom` as a `CloneFrom`. */
export function refCellCloneFromFn<T>(clone?: (value: T) => T, drop?: (item: T) => void, over?: CloneFrom<T>, at?: Location): CloneFrom<RefCell<T>> {
	return (old, src) => refCellCloneFrom(old, src, clone, drop, over, at);
}

/** A `RefMut`'s or an `RwLockWriteGuard`'s drop. */
export function refMutRelease<T>(r: RefCell<T>): void {
	refCellOf(r).borrows++;
}

/**
 * `PartialEq`, `PartialOrd`, or `Ord` of `RefCell`s, by `f` of their values: each borrowed
 * in turn, at `at` and then `other`, which panics while it is mutably borrowed, and released
 * the other way.
 */
export function refCellBy<T, R>(f: (a: T, b: T) => R, at?: Location, other?: Location): (a: RefCell<T>, b: RefCell<T>) => R {
	return (a, b) => {
		refCellBorrow(a, at);
		refCellBorrow(b, other);
		const r = f(a.value, b.value);
		refRelease(b);
		refRelease(a);
		return r;
	};
}

/** `Cell<T>`: the place that holds its value. */
export class Cell<T> implements Place<T>, Disposable {
	v: T;

	constructor(v: T) {
		this.v = v;
	}

	[Symbol.dispose](): void {
		drop(this.v);
	}
}

/** `Debug` of a `Cell` of a `Copy` value, which `get` gives once its name is written, as std reads it, since a writer may set the cell. */
export function cellDebug<T>(get: () => T, f: Formatter, debug: Debug<T>): FmtResult {
	const b = fmtDebugStruct(f, "Cell");
	const value = get();
	return debugStructFinish(debugStructField(b, "value", (f) => debug(value, f)));
}

/** `Cell::from_mut` of the place `p`: a cell of it, which nothing drops, as nothing owns it. */
export function cellOfPlace<T>(p: Place<T>): Cell<T> {
	return p as Cell<T>;
}

/**
 * `Cell::from_mut` of an object a `&mut` is, which a pointer to is a place of: a read is
 * a copy, as a `Cell` only moves or copies its value out, and a write lands in place.
 */
export function cellIn<T extends object>(p: T): Cell<T> {
	const write = partPlace(p);
	return cellOfPlace(place(() => objectCopy(p), (value) => (write.v = value)));
}

/**
 * `PartialEq`, `PartialOrd`, or `Ord` of `Cell`s or places, or `PartialEq` of `OnceCell`s
 * or `OnceLock`s, by `f` of what each holds.
 */
export function cellBy<T, R>(f: (a: T, b: T) => R): (a: { v: T }, b: { v: T }) => R {
	return (a, b) => f(a.v, b.v);
}

/** `Cell::replace`: the value it held, `v` in its place. */
export function cellReplace<T>(cell: Cell<T>, v: T): T {
	const old = cell.v;
	cell.v = v;
	return old;
}

/** `Cell::update`: what `f` returns of its value, in its place. */
export function cellUpdate<T>(cell: Cell<T>, f: (v: T) => T): void {
	cell.v = f(cell.v);
}

/** `Cell::set`: `v` in place of the value it held, which `dropOld` drops after. */
export function cellSet<T>(cell: Cell<T>, v: T, dropOld?: (old: T) => void): void {
	const old = cell.v;
	cell.v = v;
	dropOld?.(old);
}

/**
 * `LocalKey<T>`: a thread-local, one value on JS's one thread, which `init` makes on first
 * use and nothing drops, as wasm32's are. A lazy key's first use from within `init` makes
 * its own, which the outer one then drops by `drop` and replaces.
 */
export class LocalKey<T> {
	readonly #init: () => T;
	/** Whether `init` is an expression's, which only a first use runs, rather than a `const`'s. */
	readonly #lazy: boolean;
	readonly #drop: ((value: T) => void) | undefined;
	#value: T | undefined;
	/** wasm32's `LazyStorage` state: 0 before `init` returns, 1 after, 2 while dropping. */
	#state = 0;

	constructor(init: () => T, lazy: boolean, drop?: (value: T) => void) {
		this.#init = init;
		this.#lazy = lazy;
		this.#drop = drop;
	}

	/** `LocalKey::with`'s value. */
	get(): T {
		if (this.#state === 1) return this.#value as T;
		return this.#initialize(this.#init());
	}

	/** Makes `value` a lazy key's first value, as `set` does without running `init`, and whether it did. */
	install(value: T): boolean {
		if (!this.#lazy || this.#state === 1) return false;
		this.#initialize(value);
		return true;
	}

	#initialize(value: T): T {
		if (this.#state === 1) {
			this.#state = 2;
			this.#drop?.(this.#value as T);
			this.#state = 0;
		}
		if (this.#state === 2) throw new Abort("Attempted to initialize thread-local while it is being dropped");
		this.#value = value;
		this.#state = 1;
		return value;
	}
}

/**
 * `LocalKey::initialize_with`, which `set` calls: `f` of `init` and the key's value, given
 * `none` for `init` if `init` became a lazy key's first value.
 */
export function localKeyInitializeWith<T, R>(key: LocalKey<T>, init: T, f: (init: T | undefined, value: T) => R): R;
export function localKeyInitializeWith<T, R>(
	key: LocalKey<T>,
	init: T,
	f: (init: T | None, value: T) => R,
	none: None,
): R;
export function localKeyInitializeWith<T, R>(
	key: LocalKey<T>,
	init: T,
	f: (init: T | None | undefined, value: T) => R,
	none?: None,
): R {
	return f(key.install(init) ? none : init, key.get());
}

/**
 * `LocalKey::set` of a `RefCell` key: `value` becomes a lazy key's first value, or else
 * replaces its value as an exclusive borrow would, after `dropOld` drops the old one.
 */
export function localKeyRefCellSet<T>(key: LocalKey<RefCell<T>>, value: T, dropOld?: (old: T) => void, at?: Location): void {
	if (key.install(new RefCell(value))) return;
	const cell = refCellBorrowMut(key.get(), at);
	dropOld?.(cell.value);
	cell.value = value;
	refMutRelease(cell);
}

/** `LocalKey::with_borrow_mut`: `f` of its value, exclusively borrowed while `f` runs. */
export function localKeyWithBorrowMut<T, R>(key: LocalKey<RefCell<T>>, f: (value: T) => R, at?: Location): R {
	const cell = refCellBorrowMut(key.get(), at);
	const out = f(cell.value);
	refMutRelease(cell);
	return out;
}

/** `localKeyWithBorrowMut` of a value JS copies, which `f` gets the place of. */
export function localKeyWithBorrowMutPlace<T, R>(key: LocalKey<RefCell<T>>, f: (value: Place<T>) => R, at?: Location): R {
	const cell = refCellBorrowMut(key.get(), at);
	const out = f(
		place(
			() => cell.value,
			(v) => {
				cell.value = v;
			},
		),
	);
	refMutRelease(cell);
	return out;
}

// Atomics, which one thread never races: each operation reads and writes as it would
// alone, and an ordering it can't take panics as std's does.

/** `atomic::Ordering`: its discriminant, 0 to 4 for `Relaxed`, `Release`, `Acquire`, `AcqRel`, and `SeqCst`. */
export type AtomicOrdering = number;

/** `Atomic<T>` of a `bool` or an integer: the place that holds it. */
export class Atomic<T> implements Place<T> {
	v: T;

	constructor(v: T) {
		this.v = v;
	}
}

/** `Atomic::load`. */
export function atomicLoad<T>(atomic: Atomic<T>, order: AtomicOrdering, at?: Location): T {
	if (order === 1) panic("there is no such thing as a release load", at);
	if (order === 3) panic("there is no such thing as an acquire-release load", at);
	return atomic.v;
}

/** `Atomic::store`. */
export function atomicStore<T>(atomic: Atomic<T>, value: T, order: AtomicOrdering, at?: Location): void {
	if (order === 2) panic("there is no such thing as an acquire store", at);
	if (order === 3) panic("there is no such thing as an acquire-release store", at);
	atomic.v = value;
}

/** `Atomic::swap`, which any ordering can do. */
export function atomicSwap<T>(atomic: Atomic<T>, value: T, _order: AtomicOrdering): T {
	const old = atomic.v;
	atomic.v = value;
	return old;
}

/** A read-modify-write such as `Atomic::fetch_add`: the value becomes `f(old, value)`, and `old` returns. */
export function atomicFetch<T, U>(atomic: Atomic<T>, f: (old: T, value: U) => T, value: U, _order: AtomicOrdering): T {
	const old = atomic.v;
	atomic.v = f(old, value);
	return old;
}

/**
 * `Atomic::compare_exchange`, and `compare_exchange_weak`, which never fails spuriously
 * on one thread. `eq` compares a wide integer, and `===` anything else.
 */
export function atomicCompareExchange<T>(
	atomic: Atomic<T>,
	current: T,
	next: T,
	_success: AtomicOrdering,
	failure: AtomicOrdering,
	eq?: (a: T, b: T) => boolean,
	at?: Location,
): Result<T, T> {
	if (failure === 3) panic("there is no such thing as an acquire-release failure ordering", at);
	if (failure === 1) panic("there is no such thing as a release failure ordering", at);
	const old = atomic.v;
	if (eq === undefined ? old !== current : !eq(old, current)) return Err(old);
	atomic.v = next;
	return Ok(old);
}

/**
 * `Atomic::try_update`: `f` of the value it loads, compare-exchanged unless `None`, and
 * again if `f` itself changed the atomic. `eq` is as `atomicCompareExchange`'s.
 */
export function atomicTryUpdate<T>(
	atomic: Atomic<T>,
	set: AtomicOrdering,
	fetch: AtomicOrdering,
	f: (value: T) => T | undefined,
	eq?: (a: T, b: T) => boolean,
	at?: Location,
): Result<T, T> {
	let prev = atomicLoad(atomic, fetch, at);
	for (let next = f(prev); next !== undefined; next = f(prev)) {
		const result = atomicCompareExchange(atomic, prev, next, set, fetch, eq, at);
		if (result.ok) return result;
		prev = result.error;
	}
	return Err(prev);
}

/** `Atomic::update`: `f` of the value it loads, compare-exchanged until it holds. */
export function atomicUpdate<T>(atomic: Atomic<T>, set: AtomicOrdering, fetch: AtomicOrdering, f: (value: T) => T, eq?: (a: T, b: T) => boolean, at?: Location): T {
	let prev = atomicLoad(atomic, fetch, at);
	for (;;) {
		const result = atomicCompareExchange(atomic, prev, f(prev), set, fetch, eq, at);
		if (result.ok) return result.value;
		prev = result.error;
	}
}

/** `atomic::fence` and `compiler_fence`, which order nothing on one thread. */
export function fence(order: AtomicOrdering, at?: Location): void {
	if (order === 0) panic("there is no such thing as a relaxed fence", at);
}

// One-time cells

/**
 * `OnceCell<T>`: the `Option<T>` it holds, whose `None`, `N`, is `undefined`, or
 * `rs.None` where `T` can already be `undefined`.
 */
export class OnceCell<T, N extends None | undefined = undefined> implements Disposable {
	v: T | N;

	// The cell's type is its place's: its first value, often its `None`, would narrow it.
	constructor(v: NoInfer<T | N>) {
		this.v = v;
	}

	[Symbol.dispose](): void {
		drop(this.v);
	}
}

/**
 * `OnceLock<T>`, which one thread never races, but which knows it is being initialized:
 * initializing it again from within panics, as wasm32's `Once` does.
 */
export class OnceLock<T, N extends None | undefined = undefined> extends OnceCell<T, N> {
	running = false;
}

/** A `OnceLock` initialized from within its own initialization panics, as wasm32's `Once` does. */
function recursive(cell: OnceCell<unknown, None | undefined>, caller?: Location): void {
	if (cell instanceof OnceLock && cell.running) panic("one-time initialization may not be performed recursively", caller ?? "once_force");
}

/** `OnceCell::set`: `Err(value)` back if it holds one already. */
export function onceSet<T>(cell: OnceCell<T>, value: T): Result<undefined, T>;
export function onceSet<T>(cell: OnceCell<T, None>, value: T, none: None): Result<undefined, T>;
export function onceSet<T>(cell: OnceCell<T, None | undefined>, value: T, none?: None): Result<undefined, T> {
	recursive(cell);
	if (cell.v !== none) return Err(value);
	cell.v = value;
	return Ok(undefined);
}

/** `OnceCell::take`: what it holds, leaving it empty. */
export function onceTake<T>(cell: OnceCell<T>): T | undefined;
export function onceTake<T>(cell: OnceCell<T, None>, none: None): T | None;
export function onceTake<T>(cell: OnceCell<T, None | undefined>, none?: None): T | None | undefined {
	const v = cell.v;
	cell.v = none;
	return v;
}

/**
 * `OnceCell::get_or_init`: what it holds, which `f` makes first if it holds nothing,
 * dropping `f` by `dropF` if not. A `OnceLock` initialized again from within `f` panics
 * as `recursive` does, at `caller` for `Once::call_once`, which passes its caller's, and a
 * `OnceCell` that `f` sets itself panics, as std's do. `f` of a `()` returns `void`.
 */
export function onceGetOrInit<T>(cell: OnceCell<T>, f: () => T | void, dropF?: (f: () => T | void) => void, none?: undefined, caller?: Location): T;
export function onceGetOrInit<T>(cell: OnceCell<T, None>, f: () => T | void, dropF: ((f: () => T | void) => void) | undefined, none: None, caller?: Location): T;
export function onceGetOrInit<T>(cell: OnceCell<T, None | undefined>, f: () => T | void, dropF?: (f: () => T | void) => void, none?: None, caller?: Location): T {
	if (cell.v !== none) {
		dropF?.(f);
		return cell.v as T;
	}
	recursive(cell, caller);
	const lock = cell instanceof OnceLock;
	if (lock) cell.running = true;
	const value = f() as T;
	if (lock) cell.running = false;
	else if (cell.v !== none) panic("reentrant init", "reentrant_init");
	cell.v = value;
	return value;
}

/**
 * `OnceCell::get_or_try_init`: as `onceGetOrInit`, but an error from `f` leaves the cell
 * empty and is given back, before a `OnceCell` that `f` set would panic.
 */
export function onceGetOrTryInit<T, E>(cell: OnceCell<T>, f: () => Result<T, E>, dropF?: (f: () => Result<T, E>) => void): Result<T, E>;
export function onceGetOrTryInit<T, E>(cell: OnceCell<T, None>, f: () => Result<T, E>, dropF: ((f: () => Result<T, E>) => void) | undefined, none: None): Result<T, E>;
export function onceGetOrTryInit<T, E>(cell: OnceCell<T, None | undefined>, f: () => Result<T, E>, dropF?: (f: () => Result<T, E>) => void, none?: None): Result<T, E> {
	if (cell.v !== none) {
		dropF?.(f);
		return Ok(cell.v as T);
	}
	recursive(cell);
	const lock = cell instanceof OnceLock;
	if (lock) cell.running = true;
	const result = f();
	if (lock) cell.running = false;
	if (!result.ok) return result;
	if (!lock && cell.v !== none) panic("reentrant init", "reentrant_init");
	cell.v = result.value;
	return result;
}

/** `Clone` of a `OnceCell` or a `OnceLock`: one holding a clone by `clone` of what it holds, or that itself if `clone` is undefined. */
export function onceClone<T, C extends OnceCell<T>>(cell: C, clone: ((value: T) => T) | undefined): C;
export function onceClone<T, C extends OnceCell<T, None>>(cell: C, clone: ((value: T) => T) | undefined, none: None): C;
export function onceClone<T>(cell: OnceCell<T, None | undefined>, clone: ((value: T) => T) | undefined, none?: None): OnceCell<T, None | undefined> {
	const v = cell.v === none || !clone ? cell.v : clone(cell.v as T);
	return cell instanceof OnceLock ? new OnceLock(v) : new OnceCell(v);
}

/** `Debug` of a `OnceCell` or a `OnceLock`, named `name`: what it holds, or `<uninit>`. */
export function onceDebug<T>(cell: OnceCell<T>, f: Formatter, name: string, debug: Debug<T>): FmtResult;
export function onceDebug<T>(cell: OnceCell<T, None>, f: Formatter, name: string, debug: Debug<T>, none: None): FmtResult;
export function onceDebug<T>(cell: OnceCell<T, None | undefined>, f: Formatter, name: string, debug: Debug<T>, none?: None): FmtResult {
	return heldDebug(f, name, () => cell.v !== none, () => cell.v as T, debug);
}

/**
 * `Debug` of a one-time cell named `name`: its `value` if it is `held`, or else `<uninit>`,
 * unpadded. Both are read between writing the name and `(`, as std reads them, since a
 * writer may set the cell from either.
 */
function heldDebug<T>(f: Formatter, name: string, held: () => boolean, value: () => T, debug: Debug<T>): FmtResult {
	const tuple = fmtDebugTuple(f, name);
	let field = (f: Formatter) => f.out.writeStr("<uninit>");
	if (held()) {
		const v = value();
		field = (f: Formatter) => debug(v, f);
	}
	return debugTupleFinish(debugTupleField(tuple, field));
}

/** `Once`: a `OnceLock` that holds `true` once its function has run. */
export type Once = OnceLock<true>;

/** `OnceState`: the `Once` running the function given it, which nothing poisons, as a panic aborts. */
export type OnceState = Once;

/**
 * `Once::call_once` and `call_once_force`: runs `f` if `once` has not run one, or else
 * drops it by `dropF`. Run again from within `f`, it panics as `recursive` does.
 */
export function onceCall(once: Once, f: (state: OnceState) => void, dropF?: (f: (state: OnceState) => void) => void, at?: Location): void {
	if (once.v) dropF?.(f);
	else onceGetOrInit(once, () => (f(once), true), undefined, undefined, at);
}

/** `OnceState::is_poisoned`: never, as a panic aborts rather than poison a `Once`. */
export function onceIsPoisoned(_state: OnceState): boolean {
	return false;
}

/**
 * `LazyCell<T>`: the function that makes its value until the first `force` runs it, once,
 * and then that value. Forced from within that function, it panics as std's does.
 */
export class LazyCell<T> {
	#f: (() => T) | undefined;
	#value: T | undefined;
	/** Whether `force` made the value, which `undefined` can't tell where `T` can be it. */
	#init = false;

	constructor(f: () => T) {
		this.#f = f;
	}

	/** Moves its state into `to`, or into a new cell of its class. */
	$move(to?: this): this {
		const into = to ?? new (this.constructor as new (f: (() => T) | undefined) => this)(this.#f);
		into.#f = this.#f;
		into.#value = this.#value;
		into.#init = this.#init;
		return into;
	}

	/** `force` and `Deref::deref`: the value, which the function makes the first time. */
	force(): T {
		if (this.#init) return this.#value as T;
		const f = this.#f;
		if (f === undefined) this.recursive();
		this.#f = undefined;
		this.#value = f();
		this.#init = true;
		return this.#value;
	}

	/** Panics as a `force` from within the function does. */
	protected recursive(): never {
		panic("LazyCell instance has previously been poisoned", "lazy_poisoned");
	}

	/** `Debug`, named `name`: its value, or `<uninit>` until `force` makes it, which this never runs. */
	debug(f: Formatter, name: string, debug: Debug<T>): FmtResult {
		return heldDebug(f, name, () => this.#init, () => this.#value as T, debug);
	}

	/** `drop`: of its value, or of the function if `force` never ran it. */
	drop(dropValue?: (value: T) => void, dropF?: (f: () => T) => void): void {
		if (this.#init) dropValue?.(this.#value as T);
		else if (this.#f !== undefined) dropF?.(this.#f);
	}
}

/** `LazyLock<T>`, which panics as wasm32's `Once` does when forced from within its function. */
export class LazyLock<T> extends LazyCell<T> {
	protected override recursive(): never {
		panic("one-time initialization may not be performed recursively", "once_force");
	}
}

/** Drops a `LazyCell` or `LazyLock` by `dropValue` and `dropF`. */
export function lazyDrop<T>(lazy: LazyCell<T>, dropValue?: (value: T) => void, dropF?: (f: () => T) => void): void {
	lazy.drop(dropValue, dropF);
}

// Futures

/** `Poll::Pending`; `Poll::Ready(v)` is `v` itself. */
export const Pending: unique symbol = /* @__PURE__ */ Symbol("Pending");

/** `Poll<T>`. */
export type Poll<T> = T | typeof Pending;

/** `Poll::Pending` of a `Poll` whose `Ready` can already be `Pending`, as `Poll<Poll<T>>`'s is. */
export const OuterPending: unique symbol = /* @__PURE__ */ Symbol("OuterPending");

/** The type of `OuterPending`. */
export type OuterPending = typeof OuterPending;

/** A generic type's `Poll<T>`, whose `Pending` each instance decides: `Poll<T>`, or `T | OuterPending` where `T` can already be `Pending`. */
export type PollOf<T> = T | (typeof Pending extends T ? OuterPending : typeof Pending);

/** `Debug` of a `Poll`, through its value's, whose `Pending` is `pending`. */
export function fmtPoll<T>(p: T | typeof Pending | OuterPending, f: Formatter, debug: Debug<T>, pending: typeof Pending | OuterPending = Pending): FmtResult {
	if (p === pending) return f.out.writeStr("Pending");
	return fmtDebugTupleFields(f, "Ready", [(f) => debug(p as T, f)]);
}

/** `Hash::hash` of a `Poll`, whose `Pending` is `pending`: its discriminant as an `isize`, then its value's. */
export function hashPoll<T>(hash: (value: T, s: Hasher) => void, pending: typeof Pending | OuterPending = Pending): (value: T | typeof Pending | OuterPending, s: Hasher) => void {
	return (value, s) => {
		s.isize(value === pending ? 1 : 0);
		if (value !== pending) hash(value as T, s);
	};
}

/** `Ok(Pending)` and `Ok(OuterPending)`, shared as `OK` is. */
const OK_PENDING = /* @__PURE__ */ Object.freeze({ ok: true, value: Pending } as const);
const OK_OUTER_PENDING = /* @__PURE__ */ Object.freeze({ ok: true, value: OuterPending } as const);

/**
 * `?` on a `Poll<Result<T, E>>`: the `Result` it is when ready, and `Ok(pending)` when
 * `Pending`, where `pending` is the `Pending` of the `Poll<T>` it continues with.
 */
export function pollBranch<T, E, P extends typeof Pending | OuterPending>(
	poll: Result<T, E> | typeof Pending,
	pending: P,
): Result<T | P, E> {
	if (poll !== Pending) return poll;
	return (pending === Pending ? OK_PENDING : OK_OUTER_PENDING) as Result<P, E>;
}

/**
 * `?` on a `Poll<Option<Result<T, E>>>`, as `pollBranch`, whose `Ready(None)` continues as
 * `Ok(none)`, where `none` is the `None` of the `Option<T>`.
 */
export function pollOptionBranch<T, E, P extends typeof Pending | OuterPending, N extends undefined | None>(
	poll: Result<T, E> | undefined | typeof Pending,
	pending: P,
	none: N,
): Result<T | P | N, E> {
	if (poll === undefined) return none === undefined ? (OK as Result<N, E>) : Ok(none);
	return pollBranch(poll, pending);
}

/**
 * `Waker`: what an executor hands a future to wake its task. `data` and `vtable` are its
 * identity, std's `Waker::data` and `Waker::vtable`, which each of its clones shares, and
 * which `Waker::new` makes a waker of the same task from.
 */
export interface Waker extends Disposable {
	readonly data: unknown;
	readonly vtable: WakerVTable;
	wake(): void;
	wakeByRef(): void;
	clone(): Waker;
}

/**
 * A waker's `&'static RawWakerVTable`: what clones, wakes, and drops its data, under std's
 * names. `will_wake` compares its identity, and `==` its functions.
 */
export interface WakerVTable {
	readonly clone: (data: Place<undefined>) => RawWaker;
	readonly wake: (data: Place<undefined>) => void;
	readonly wakeByRef: (data: Place<undefined>) => void;
	readonly drop: (data: Place<undefined>) => void;
}

/** `RawWakerVTable::new`. */
export function wakerVTable(
	clone: (data: Place<undefined>) => RawWaker,
	wake: (data: Place<undefined>) => void,
	wakeByRef: (data: Place<undefined>) => void,
	drop: (data: Place<undefined>) => void,
): WakerVTable {
	return { clone, wake, wakeByRef, drop };
}

/** `RawWaker`: a waker's data and vtable, under std's names. */
export interface RawWaker {
	readonly data: Place<undefined>;
	readonly vtable: WakerVTable;
}

/** `a.will_wake(b)`: whether their identities are the same, as std compares them. */
export function willWake(a: Waker, b: Waker): boolean {
	return a.data === b.data && a.vtable === b.vtable;
}

/**
 * `Waker::clone_from` of `src` over `old`: `old`, where it would wake what `src` would, as
 * std's keeps it without a clone or a drop, and else `src`'s clone, made before `old` drops.
 */
export function wakerCloneFrom(old: Waker, src: Waker): Waker {
	if (willWake(old, src)) return old;
	return cloneOver(old, src, (w) => w.clone(), (w) => w[Symbol.dispose]());
}

/** `wakerCloneFrom` as a `CloneFrom`. */
export function wakerCloneFromFn(): CloneFrom<Waker> {
	return wakerCloneFrom;
}

/** `Waker::new(data, vtable)`: a waker that calls `vtable`'s functions on `data`, as std's does. */
export class VTableWaker implements Waker {
	readonly data: Place<undefined>;
	readonly vtable: WakerVTable;

	constructor(data: Place<undefined>, vtable: WakerVTable) {
		this.data = data;
		this.vtable = vtable;
	}

	wake(): void {
		this.vtable.wake(this.data);
	}

	wakeByRef(): void {
		this.vtable.wakeByRef(this.data);
	}

	clone(): Waker {
		return wakerFromRaw(this.vtable.clone(this.data));
	}

	[Symbol.dispose](): void {
		this.vtable.drop(this.data);
	}
}

/** `Waker::from_raw(raw)`. */
export function wakerFromRaw(raw: RawWaker): Waker {
	return new VTableWaker(raw.data, raw.vtable);
}

/** Each `Wake` impl's vtable, by its `wake`, as std makes one per impl. */
const ARC_VTABLES = /* @__PURE__ */ new WeakMap<object, WakerVTable>();

/**
 * `RawWaker::from(arc)`: data that is `arc`, whose count the waker holds, and the vtable of
 * `arc`'s `Wake` impl, whose `wake` takes that count. Without a `wakeByRef` of its own, one
 * wakes a clone, as `Wake`'s default does. `arc` is every clone of `arc`, as std's data is
 * where they all point.
 */
export function arcRawWaker<W>(
	arc: Arc<W>,
	wake: (arc: Arc<W>) => void,
	drop: (arc: Arc<W>) => void,
	wakeByRef?: (arc: Arc<W>) => void,
): RawWaker {
	let vtable = ARC_VTABLES.get(wake);
	if (vtable === undefined) {
		const of = (data: Place<undefined>) => data as unknown as Arc<W>;
		vtable = wakerVTable(
			(data) => ({ data: arcClone(of(data)) as unknown as Place<undefined>, vtable: vtable! }),
			(data) => wake(of(data)),
			(data) => (wakeByRef === undefined ? wake(arcClone(of(data))) : wakeByRef(of(data))),
			(data) => drop(of(data)),
		);
		ARC_VTABLES.set(wake, vtable);
	}
	return { data: arc as unknown as Place<undefined>, vtable };
}

/** `Waker::from(arc)`: the waker of `arcRawWaker`'s. */
export function arcWaker<W>(
	arc: Arc<W>,
	wake: (arc: Arc<W>) => void,
	drop: (arc: Arc<W>) => void,
	wakeByRef?: (arc: Arc<W>) => void,
): Waker {
	return wakerFromRaw(arcRawWaker(arc, wake, drop, wakeByRef));
}

/** The vtable of `Waker::noop()`, whose functions do nothing. */
const NOOP_VTABLE: WakerVTable = /* @__PURE__ */ wakerVTable(
	() => ({ data: NULL, vtable: NOOP_VTABLE }),
	() => {},
	() => {},
	() => {},
);

/** `Waker::noop()`: one waker for every call, whose data is null, as std's is. */
// `rs.NULL`, which `rs.ts` makes after this module runs.
export const NOOP_WAKER: Waker = /* @__PURE__ */ new VTableWaker(null as never, NOOP_VTABLE);

/** `Context`, which carries only its waker. */
export type Context = Waker;

/** A future, as executors poll it. Dropping it cancels it. */
export interface Future<T> extends Disposable {
	poll(cx: Context): Poll<T>;
}

/**
 * `future::poll_fn`'s future: each poll calls `f`, which drops with it by `drop`, if it
 * drops at all: a borrowed `f` does not.
 */
export class PollFn<T> implements Future<T> {
	#f: (cx: Context) => Poll<T>;
	#drop: ((f: (cx: Context) => Poll<T>) => void) | undefined;

	constructor(f: (cx: Context) => Poll<T>, drop?: (f: (cx: Context) => Poll<T>) => void) {
		this.#f = f;
		this.#drop = drop;
	}

	/** Moves its function into `to`, or into a new future. */
	$move(to?: this): this {
		if (to === undefined) return new PollFn(this.#f, this.#drop) as this;
		to.#f = this.#f;
		to.#drop = this.#drop;
		return to;
	}

	poll(cx: Context): Poll<T> {
		return this.#f(cx);
	}

	[Symbol.dispose](): void {
		this.#drop?.(this.#f);
	}
}

/**
 * A `dyn Future` of a type whose `Future` impl is a function: each poll calls `poll` on
 * `value`, which drops with it by `drop`, if it drops at all: a borrowed one does not.
 */
export class DynFuture<S, T> implements Future<T> {
	readonly #value: S;
	readonly #poll: (value: S, cx: Context) => Poll<T>;
	readonly #drop: ((value: S) => void) | undefined;

	constructor(value: S, poll: (value: S, cx: Context) => Poll<T>, drop?: (value: S) => void) {
		this.#value = value;
		this.#poll = poll;
		this.#drop = drop;
	}

	poll(cx: Context): Poll<T> {
		return this.#poll(this.#value, cx);
	}

	[Symbol.dispose](): void {
		this.#drop?.(this.#value);
	}
}

/** `future.poll(cx)`: the `poll` of an `rs.DynFuture` over a future it borrows. */
export function futurePoll<T>(future: Future<T>, cx: Context): Poll<T> {
	return future.poll(cx);
}

/**
 * `future::ready(value)`'s future: its first poll gives `value`, and a later one panics,
 * as `into_inner` after it does. A value no poll took drops with it by `drop`, if at all.
 */
export class Ready<T> implements Future<T> {
	#value: T | undefined;
	#taken: boolean;
	readonly #drop: ((value: T) => void) | undefined;

	constructor(value: T, drop?: (value: T) => void, taken = false) {
		this.#value = value;
		this.#taken = taken;
		this.#drop = drop;
	}

	/** Moves its value into `to`, or into a new future. */
	$move(to?: this): this {
		if (to === undefined) return new Ready(this.#value as T, this.#drop, this.#taken) as this;
		to.#value = this.#value;
		to.#taken = this.#taken;
		return to;
	}

	poll(_cx: Context): Poll<T> {
		return this.#take("`Ready` polled after completion", "ready_poll");
	}

	/** `into_inner`, which consumes it. */
	intoInner(): T {
		return this.#take("Called `into_inner()` on `Ready` after completion", "ready_into_inner");
	}

	#take(message: string, at: string): T {
		if (this.#taken) panic(message, at, true);
		const value = this.#value as T;
		this.#taken = true;
		this.#value = undefined;
		return value;
	}

	/** `clone`, of its value by `clone` unless it copies, or of none once taken. */
	clone(clone?: (value: T) => T): Ready<T> {
		if (this.#taken) return new Ready(undefined as T, this.#drop, true);
		return new Ready(clone ? clone(this.#value as T) : (this.#value as T), this.#drop);
	}

	/** `Debug`, std's derived one of the `Option` it holds. */
	debug(f: Formatter, debug: Debug<T>): FmtResult {
		const held = (f: Formatter): FmtResult =>
			this.#taken ? f.out.writeStr("None") : fmtDebugTupleFields(f, "Some", [(f) => debug(this.#value as T, f)]);
		return fmtDebugTupleFields(f, "Ready", [held]);
	}

	[Symbol.dispose](): void {
		if (!this.#taken) this.#drop?.(this.#value as T);
	}
}

/** `future::Pending`: never ready, and drops as nothing. */
class Never implements Future<never> {
	poll(): Poll<never> {
		return Pending;
	}

	[Symbol.dispose](): void {}
}

/** `future::pending()`: one future for every call, since none holds anything. */
export const NEVER: Future<never> = /* @__PURE__ */ new Never();

/** An `async fn` polled again after it returned. */
export function asyncFnResumed(at?: Location): never {
	panic("`async fn` resumed after completion", at);
}

/** An `async fn` polled again after it panicked. */
export function asyncFnResumedPanic(at?: Location): never {
	panic("`async fn` resumed after panicking", at);
}

// Executor

/**
 * Run `future` to completion on the microtask queue, as a promise of its output. A
 * panic rejects the promise with the `Panic`. Aborting `signal` cancels it: the future
 * drops as Rust drops a future, at once, or as the poll it aborts in returns pending,
 * and the promise rejects with the signal's reason. A future that is ready first wins.
 */
export function spawn<T>(future: Future<T>, signal?: AbortSignal): Promise<T> {
	return new Promise((resolve, reject) => new Task(future, resolve, reject, signal).schedule());
}

/** How many polls one microtask runs before it yields to a macrotask, so a task that keeps waking itself cannot starve I/O. */
const BUDGET = 128;

/** Tasks woken since the last run, in order. */
const queue: { run(): void }[] = [];
let scheduled = false;

// `scheduled` stays set while a batch runs, so a task it wakes joins the batch rather
// than queueing a microtask that would run ahead of the macrotask.
function run(): void {
	try {
		for (let polls = 0; polls < BUDGET && queue.length > 0; polls++) queue.shift()!.run();
	} finally {
		if (queue.length > 0) setTimeout(run, 0);
		else scheduled = false;
	}
}

/** The vtable of every spawned task's waker, whose data is the task. */
const TASK_VTABLE: WakerVTable = /* @__PURE__ */ wakerVTable(
	(data) => ({ data, vtable: TASK_VTABLE }),
	(data) => (data as unknown as Task<unknown>).schedule(),
	(data) => (data as unknown as Task<unknown>).schedule(),
	() => {},
);

/** A spawned future, which is also the waker of every poll it gets: its data. */
class Task<T> implements Waker {
	#future: Future<T> | undefined;
	#resolve: (value: T) => void;
	#reject: (err: unknown) => void;
	#queued = false;
	#signal: AbortSignal | undefined;
	/** Whether a poll is running, which an abort waits out, as Rust can't drop a future it polls. */
	#polling = false;

	constructor(future: Future<T>, resolve: (value: T) => void, reject: (err: unknown) => void, signal?: AbortSignal) {
		this.#future = future;
		this.#resolve = resolve;
		this.#reject = reject;
		if (signal === undefined) return;
		this.#signal = signal;
		if (signal.aborted) this.#cancel();
		else signal.addEventListener("abort", this);
	}

	/** The signal's abort: cancel now, or once the running poll returns. */
	handleEvent(): void {
		if (!this.#polling) this.#cancel();
	}

	/** Drop the future and reject with the signal's reason, or with a panic its drop throws. */
	#cancel(): void {
		const future = this.#future;
		if (future === undefined) return;
		this.#finish();
		try {
			future[Symbol.dispose]();
		} catch (err) {
			this.#reject(err);
			if (!(err instanceof Panic)) throw err;
			return;
		}
		this.#reject(this.#signal!.reason);
	}

	/** Settle: no poll, wake, or abort reaches the future again. */
	#finish(): void {
		this.#future = undefined;
		this.#signal?.removeEventListener("abort", this);
	}

	schedule(): void {
		if (this.#queued || this.#future === undefined) return;
		this.#queued = true;
		queue.push(this);
		if (!scheduled) {
			scheduled = true;
			queueMicrotask(run);
		}
	}

	run(): void {
		this.#queued = false;
		const future = this.#future;
		if (future === undefined) return;
		let output: Poll<T>;
		try {
			this.#polling = true;
			output = future.poll(this);
			this.#polling = false;
			if (output !== Pending) {
				this.#finish();
				future[Symbol.dispose]();
			}
		} catch (err) {
			this.#polling = false;
			this.#finish();
			this.#reject(err);
			if (!(err instanceof Panic)) throw err;
			return;
		}
		if (output !== Pending) this.#resolve(output);
		else if (this.#signal?.aborted) this.#cancel();
	}

	wake(): void {
		this.schedule();
	}

	wakeByRef(): void {
		this.schedule();
	}

	get data(): unknown {
		return this;
	}

	get vtable(): WakerVTable {
		return TASK_VTABLE;
	}

	clone(): Waker {
		return this;
	}

	[Symbol.dispose](): void {}
}
