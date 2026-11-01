// Ownership and async, for rust2-ts output: what Rust's `Drop`, `Arc`, `Mutex`, atomics,
// one-time cells, and futures do that plain JS does not. `rt.ts` re-exports all of it.
//
// A value whose drop JS can observe has `[Symbol.dispose]`, which runs its `Drop` impl
// and drops its fields. `None` is `undefined`, which drops as nothing.

import { type Debug, debugTupleField, debugTupleFinish, Err, type FmtResult, type Formatter, fmtDebugTuple, fmtPad, type None, OK, Ok, Panic, panic, type Place, place, type Result } from "../rt.ts";

// Drops

/** Drop `value`: run its `[Symbol.dispose]`, if it has one. */
export function drop(value: unknown): void {
	(value as Partial<Disposable> | null | undefined)?.[Symbol.dispose]?.();
}

/** Drop every element of `values` from `from` on, in order, by `f` if it has no `[Symbol.dispose]`. */
export function dropAll<T>(values: ArrayLike<T> | undefined, f: (value: T) => void = drop, from = 0): void {
	if (values !== undefined) dropRange(values, f, from, values.length);
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

/** `FnOnce::call_once` of a closure that `Fn` or `FnMut` call: `f(...args)`, then `f` drops. */
export function callOnce<A extends unknown[], R>(f: (...args: A) => R, ...args: A): R {
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
	return arc instanceof ArcDyn ? arc.arc : arc;
}

/** `Arc::downcast` of an `Arc<dyn Any>`: the `Arc` it was made from, if its object is a `class`. */
export function arcDowncast<T, U>(arc: Arc<U>, class_: abstract new (...args: never[]) => unknown): Result<Arc<T>, Arc<U>> {
	return arc instanceof ArcDyn && arc.value instanceof class_ ? Ok(arc.arc as Arc<T>) : Err(arc);
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
 * sharing it leave them dangling as the value moves to a new `Arc`.
 */
export function arcMakeMut<T>(p: Place<Arc<T>>, clone: ((value: T) => T) | undefined, drop: (arc: Arc<T>) => void): T {
	return arcOwn(p, clone, drop).value;
}

/** `Arc::make_mut` of a value JS copies: where it is. */
export function arcMakeMutPlace<T>(
	p: Place<Arc<T>>,
	clone: ((value: T) => T) | undefined,
	drop: (arc: Arc<T>) => void,
): Place<T> {
	return valuePlace(arcOwn(p, clone, drop));
}

function arcUnique(arc: Arc<unknown>): boolean {
	return arc.strong === 1 && arc.weak === 0;
}

function arcOwn<T>(p: Place<Arc<T>>, clone: ((value: T) => T) | undefined, drop: (arc: Arc<T>) => void): Arc<T> {
	const arc = p.v;
	if (arc.strong !== 1) {
		p.v = new Arc(clone ? clone(arc.value) : arc.value);
		drop(arc);
	} else if (arc.weak !== 0) {
		arc.strong = 0;
		p.v = new Arc(arc.value);
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

/** `Weak<T>`, of `arc`, or else dangling, counted in `arc` until it drops. */
export class Weak<T> implements Disposable {
	readonly arc: Arc<T>;

	constructor(arc: Arc<T> = DANGLING) {
		this.arc = arc;
		arc.weak++;
	}

	[Symbol.dispose](): void {
		this.arc.weak--;
	}
}

/** `Weak::upgrade`: a new `Arc`, or `undefined` once none is left. */
export function weakUpgrade<T>(weak: Weak<T>): Arc<T> | undefined {
	return weak.arc.strong > 0 ? arcClone(weak.arc) : undefined;
}

/** `Weak::clone`. */
export function weakClone<T>(weak: Weak<T>): Weak<T> {
	return new Weak(weak.arc);
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

/** `TryLockError<T>`, tagged as the crate's own enums are; a panic aborts, so it is never `Poisoned`. */
export type TryLockError<T> = { readonly tag: "Poisoned"; readonly _0: T } | { readonly tag: "WouldBlock" };

const WOULD_BLOCK: Result<never, TryLockError<never>> = /* @__PURE__ */ Err({ tag: "WouldBlock" });

/** `Mutex::lock`: its guard, which is the mutex. Locking it again while held panics, as wasm32's does. */
export function mutexLock<T>(mutex: Mutex<T>): Mutex<T> {
	if (mutex.locked) panic("assertion `left == right` failed: cannot recursively acquire mutex\n  left: true\n right: false");
	mutex.locked = true;
	return mutex;
}

/** `Mutex::try_lock`: its guard, or `WouldBlock` while another guard holds it. */
export function mutexTryLock<T>(mutex: Mutex<T>): Result<Mutex<T>, TryLockError<Mutex<T>>> {
	if (mutex.locked) return WOULD_BLOCK;
	mutex.locked = true;
	return Ok(mutex);
}

/** A `MutexGuard`'s drop. */
export function mutexUnlock<T>(mutex: Mutex<T>): void {
	mutex.locked = false;
}

/**
 * `RefCell<T>` and `RwLock<T>`, which one thread can't tell apart but by how a borrow
 * that conflicts fails, and their guards: a borrow is the cell itself, `borrows` counting
 * the shared ones held, or -1 while an exclusive one is.
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
export function refCellBorrow<T>(cell: RefCell<T>): RefCell<T> {
	if (cell.borrows < 0) panic("RefCell already mutably borrowed");
	cell.borrows++;
	return cell;
}

/** `RefCell::borrow_mut`: an exclusive borrow, which panics while any other is held. */
export function refCellBorrowMut<T>(cell: RefCell<T>): RefCell<T> {
	if (cell.borrows !== 0) panic("RefCell already borrowed");
	cell.borrows = -1;
	return cell;
}

/** `RefCell::replace`: its value, which `value` replaces, as an exclusive borrow would. */
export function refCellReplace<T>(cell: RefCell<T>, value: T): T {
	if (cell.borrows !== 0) panic("RefCell already borrowed");
	const old = cell.value;
	cell.value = value;
	return old;
}

/** `RwLock::read`: a shared guard, which aborts while a write is held, as wasm32's does. */
export function rwLockRead<T>(lock: RefCell<T>): RefCell<T> {
	if (lock.borrows < 0) panic("rwlock locked for writing");
	lock.borrows++;
	return lock;
}

/** `RwLock::write`: an exclusive guard, which aborts while any other is held, as wasm32's does. */
export function rwLockWrite<T>(lock: RefCell<T>): RefCell<T> {
	if (lock.borrows !== 0) panic("rwlock locked for reading");
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

/**
 * A `Ref` that `Ref::map` made: the part of a cell's value it lends, whose borrow that
 * cell counts.
 */
export class RefMap<T> extends RefCell<T> {
	readonly cell: RefCell<unknown>;

	constructor(value: T, cell: RefCell<unknown>) {
		super(value);
		this.cell = cell;
	}
}

/** The cell whose borrow `r`, a `Ref`, holds. */
function refCellOf(r: RefCell<unknown>): RefCell<unknown> {
	return r instanceof RefMap ? r.cell : r;
}

/** A `Ref`'s or an `RwLockReadGuard`'s drop. */
export function refRelease<T>(r: RefCell<T>): void {
	refCellOf(r).borrows--;
}

/** `Ref::map`: a `Ref` of the part of `r`'s value `f` lends, which takes over `r`'s borrow. */
export function refMap<T, U>(r: RefCell<T>, f: (value: T) => U): RefCell<U> {
	return new RefMap(f(r.value), refCellOf(r));
}

/** `Ref::clone`: `r` again, a shared borrow of its own, which never conflicts. */
export function refClone<T>(r: RefCell<T>): RefCell<T> {
	refCellOf(r).borrows++;
	return r;
}

/** `RefCell::try_borrow`'s error: the cell is mutably borrowed. */
export class BorrowError {}

/** `RefCell::try_borrow_mut`'s error: the cell is borrowed. */
export class BorrowMutError {}

const BORROW_ERROR: Result<never, BorrowError> = /* @__PURE__ */ Err(new BorrowError());
const BORROW_MUT_ERROR: Result<never, BorrowMutError> = /* @__PURE__ */ Err(new BorrowMutError());

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
export function refCellClone<T>(cell: RefCell<T>, clone?: (value: T) => T): RefCell<T> {
	refCellBorrow(cell);
	const value = clone ? clone(cell.value) : cell.value;
	refRelease(cell);
	return new RefCell(value);
}

/** A `RefMut`'s or an `RwLockWriteGuard`'s drop. */
export function refMutRelease<T>(cell: RefCell<T>): void {
	cell.borrows = 0;
}

/**
 * `PartialEq`, `PartialOrd`, or `Ord` of `RefCell`s, by `f` of their values: each borrowed
 * in turn, which panics while it is mutably borrowed, and released the other way.
 */
export function refCellBy<T, R>(f: (a: T, b: T) => R): (a: RefCell<T>, b: RefCell<T>) => R {
	return (a, b) => {
		refCellBorrow(a);
		refCellBorrow(b);
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

/**
 * `PartialEq`, `PartialOrd`, or `Ord` of `Cell`s, or `PartialEq` of `OnceCell`s or
 * `OnceLock`s, by `f` of what each holds.
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
		if (this.#state === 2) panic("Attempted to initialize thread-local while it is being dropped");
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
export function localKeyRefCellSet<T>(key: LocalKey<RefCell<T>>, value: T, dropOld?: (old: T) => void): void {
	if (key.install(new RefCell(value))) return;
	const cell = refCellBorrowMut(key.get());
	dropOld?.(cell.value);
	cell.value = value;
	refMutRelease(cell);
}

/** `LocalKey::with_borrow_mut`: `f` of its value, exclusively borrowed while `f` runs. */
export function localKeyWithBorrowMut<T, R>(key: LocalKey<RefCell<T>>, f: (value: T) => R): R {
	const cell = refCellBorrowMut(key.get());
	const out = f(cell.value);
	refMutRelease(cell);
	return out;
}

/** `localKeyWithBorrowMut` of a value JS copies, which `f` gets the place of. */
export function localKeyWithBorrowMutPlace<T, R>(key: LocalKey<RefCell<T>>, f: (value: Place<T>) => R): R {
	const cell = refCellBorrowMut(key.get());
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
export function atomicLoad<T>(atomic: Atomic<T>, order: AtomicOrdering): T {
	if (order === 1) panic("there is no such thing as a release load");
	if (order === 3) panic("there is no such thing as an acquire-release load");
	return atomic.v;
}

/** `Atomic::store`. */
export function atomicStore<T>(atomic: Atomic<T>, value: T, order: AtomicOrdering): void {
	if (order === 2) panic("there is no such thing as an acquire store");
	if (order === 3) panic("there is no such thing as an acquire-release store");
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
): Result<T, T> {
	if (failure === 3) panic("there is no such thing as an acquire-release failure ordering");
	if (failure === 1) panic("there is no such thing as a release failure ordering");
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
): Result<T, T> {
	let prev = atomicLoad(atomic, fetch);
	for (let next = f(prev); next !== undefined; next = f(prev)) {
		const result = atomicCompareExchange(atomic, prev, next, set, fetch, eq);
		if (result.ok) return result;
		prev = result.error;
	}
	return Err(prev);
}

/** `Atomic::update`: `f` of the value it loads, compare-exchanged until it holds. */
export function atomicUpdate<T>(atomic: Atomic<T>, set: AtomicOrdering, fetch: AtomicOrdering, f: (value: T) => T, eq?: (a: T, b: T) => boolean): T {
	let prev = atomicLoad(atomic, fetch);
	for (;;) {
		const result = atomicCompareExchange(atomic, prev, f(prev), set, fetch, eq);
		if (result.ok) return result.value;
		prev = result.error;
	}
}

/** `atomic::fence` and `compiler_fence`, which order nothing on one thread. */
export function fence(order: AtomicOrdering): void {
	if (order === 0) panic("there is no such thing as a relaxed fence");
}

// One-time cells

/**
 * `OnceCell<T>`: the `Option<T>` it holds, whose `None`, `N`, is `undefined`, or
 * `rt.None` where `T` can already be `undefined`.
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
function recursive(cell: OnceCell<unknown, None | undefined>): void {
	if (cell instanceof OnceLock && cell.running) panic("one-time initialization may not be performed recursively");
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
 * as `recursive` does, and a `OnceCell` that `f` sets itself panics, as std's do. `f` of
 * a `()` returns `void`.
 */
export function onceGetOrInit<T>(cell: OnceCell<T>, f: () => T | void, dropF?: (f: () => T | void) => void): T;
export function onceGetOrInit<T>(cell: OnceCell<T, None>, f: () => T | void, dropF: ((f: () => T | void) => void) | undefined, none: None): T;
export function onceGetOrInit<T>(cell: OnceCell<T, None | undefined>, f: () => T | void, dropF?: (f: () => T | void) => void, none?: None): T {
	if (cell.v !== none) {
		dropF?.(f);
		return cell.v as T;
	}
	recursive(cell);
	const lock = cell instanceof OnceLock;
	if (lock) cell.running = true;
	const value = f() as T;
	if (lock) cell.running = false;
	else if (cell.v !== none) panic("reentrant init");
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
	if (!lock && cell.v !== none) panic("reentrant init");
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
	const v = cell.v;
	const field = v === none ? (f: Formatter) => f.out.writeStr("<uninit>") : (f: Formatter) => debug(v as T, f);
	return debugTupleFinish(debugTupleField(fmtDebugTuple(f, name), field));
}

/** `Once`: a `OnceLock` that holds `true` once its function has run. */
export type Once = OnceLock<true>;

/** `OnceState`: the `Once` running the function given it, which nothing poisons, as a panic aborts. */
export type OnceState = Once;

/**
 * `Once::call_once` and `call_once_force`: runs `f` if `once` has not run one, or else
 * drops it by `dropF`. Run again from within `f`, it panics as `recursive` does.
 */
export function onceCall(once: Once, f: (state: OnceState) => void, dropF?: (f: (state: OnceState) => void) => void): void {
	if (once.v) dropF?.(f);
	else onceGetOrInit(once, () => (f(once), true));
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
		panic("LazyCell instance has previously been poisoned");
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
		panic("one-time initialization may not be performed recursively");
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
 * identity, std's `Waker::data` and `Waker::vtable`, which each of its clones shares.
 */
export interface Waker extends Disposable {
	readonly data: unknown;
	readonly vtable: WakerVTable;
	wake(): void;
	wakeByRef(): void;
	clone(): Waker;
}

/** A waker's `&'static RawWakerVTable`, of which only its identity is read. */
export type WakerVTable = object;

/** `a.will_wake(b)`: whether their identities are the same, as std compares them. */
export function willWake(a: Waker, b: Waker): boolean {
	return a.data === b.data && a.vtable === b.vtable;
}

/**
 * `Waker::from(arc)`: a waker that holds a count of `arc` and wakes through its `Wake`
 * impl, `wake` taking that count. Without a `wakeByRef` of its own, one wakes a clone, as
 * `Wake`'s default does. Its data is `arc`, which every clone of `arc` is, and its vtable
 * `wake`, one per `Wake` impl as std's vtable is.
 */
export class ArcWaker<W> implements Waker {
	readonly #arc: Arc<W>;
	readonly #wake: (arc: Arc<W>) => void;
	readonly #drop: (arc: Arc<W>) => void;
	readonly #wakeByRef: ((arc: Arc<W>) => void) | undefined;

	constructor(
		arc: Arc<W>,
		wake: (arc: Arc<W>) => void,
		drop: (arc: Arc<W>) => void,
		wakeByRef?: (arc: Arc<W>) => void,
	) {
		this.#arc = arc;
		this.#wake = wake;
		this.#drop = drop;
		this.#wakeByRef = wakeByRef;
	}

	wake(): void {
		this.#wake(this.#arc);
	}

	wakeByRef(): void {
		if (this.#wakeByRef === undefined) this.#wake(arcClone(this.#arc));
		else this.#wakeByRef(this.#arc);
	}

	get data(): unknown {
		return this.#arc;
	}

	get vtable(): WakerVTable {
		return this.#wake;
	}

	clone(): Waker {
		return new ArcWaker(arcClone(this.#arc), this.#wake, this.#drop, this.#wakeByRef);
	}

	[Symbol.dispose](): void {
		this.#drop(this.#arc);
	}
}

/** `Waker::noop()`'s waker, which does nothing: null data, as std's is. */
class NoopWaker implements Waker {
	readonly data = null;
	readonly vtable: WakerVTable = Object.freeze({});

	wake(): void {}

	wakeByRef(): void {}

	clone(): Waker {
		return this;
	}

	[Symbol.dispose](): void {}
}

/** `Waker::noop()`: one waker for every call. */
export const NOOP_WAKER: Waker = /* @__PURE__ */ new NoopWaker();

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
	readonly #f: (cx: Context) => Poll<T>;
	readonly #drop: ((f: (cx: Context) => Poll<T>) => void) | undefined;

	constructor(f: (cx: Context) => Poll<T>, drop?: (f: (cx: Context) => Poll<T>) => void) {
		this.#f = f;
		this.#drop = drop;
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

/** `future.poll(cx)`: the `poll` of an `rt.DynFuture` over a future it borrows. */
export function futurePoll<T>(future: Future<T>, cx: Context): Poll<T> {
	return future.poll(cx);
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
export function asyncFnResumed(): never {
	throw new Panic("`async fn` resumed after completion");
}

/** An `async fn` polled again after it panicked. */
export function asyncFnResumedPanic(): never {
	throw new Panic("`async fn` resumed after panicking");
}

// Executor

/**
 * Run `future` to completion on the microtask queue, as a promise of its output. A
 * panic rejects the promise with the `Panic`.
 */
export function spawn<T>(future: Future<T>): Promise<T> {
	return new Promise((resolve, reject) => new Task(future, resolve, reject).schedule());
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

/** The vtable of every spawned task's waker. */
const TASK_VTABLE: WakerVTable = /* @__PURE__ */ Object.freeze({});

/** A spawned future, which is also the waker of every poll it gets: its data. */
class Task<T> implements Waker {
	#future: Future<T> | undefined;
	#resolve: (value: T) => void;
	#reject: (err: unknown) => void;
	#queued = false;

	constructor(future: Future<T>, resolve: (value: T) => void, reject: (err: unknown) => void) {
		this.#future = future;
		this.#resolve = resolve;
		this.#reject = reject;
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
			output = future.poll(this);
			if (output === Pending) return;
			this.#future = undefined;
			future[Symbol.dispose]();
		} catch (err) {
			this.#future = undefined;
			this.#reject(err);
			if (!(err instanceof Panic)) throw err;
			return;
		}
		this.#resolve(output);
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
