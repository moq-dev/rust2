//! The runtime the Python glue's extension functions share: the conversions between
//! Python's values and the C ABI's, the handles objects own, and the queue a future's
//! and a waker's wakes reach the event loop through.
//!
//! The glue holds the GIL only while it touches Python: it releases it around each call
//! into the C ABI, so Rust may call Python back from any thread, as a lent closure or a
//! trait method, while the caller waits. A wake never takes the GIL: it queues a token
//! and writes a byte to a socket the event loop reads, and the loop does the rest.

use std::cell::UnsafeCell;
use std::ffi::{CStr, c_char, c_int, c_void};
use std::io::{Read, Write};
use std::ptr::{NonNull, null, null_mut};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicPtr, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

pub use pyo3_ffi::*;

use crate::rt;

/// A Python exception is set, for the caller to return null with.
pub struct Raised;

pub type R<T> = Result<T, Raised>;

/// A value only the thread holding the GIL touches.
pub struct Gil<T>(UnsafeCell<T>);

// SAFETY: every access holds the GIL, which this abi3 extension's interpreters have.
unsafe impl<T> Sync for Gil<T> {}

impl<T> Gil<T> {
	pub const fn new(value: T) -> Self {
		Self(UnsafeCell::new(value))
	}

	/// The value.
	///
	/// # Safety
	/// The caller holds the GIL, and no other reference to the value is live.
	#[allow(clippy::mut_from_ref, reason = "the GIL makes the access exclusive")]
	pub unsafe fn get(&self) -> &mut T {
		// SAFETY: the caller's contract.
		unsafe { &mut *self.0.get() }
	}
}

/// A strong reference, released when dropped.
pub struct Obj(NonNull<PyObject>);

impl Obj {
	/// A new reference a call returned, or the exception it raised.
	///
	/// # Safety
	/// `ptr` is null with an exception set, or a new reference, and the GIL is held.
	pub unsafe fn new(ptr: *mut PyObject) -> R<Self> {
		NonNull::new(ptr).map(Self).ok_or(Raised)
	}

	/// A new reference to `ptr`, which the caller borrows.
	///
	/// # Safety
	/// `ptr` is a live object, and the GIL is held.
	pub unsafe fn borrowed(ptr: *mut PyObject) -> Self {
		// SAFETY: the caller's contract.
		unsafe { Py_IncRef(ptr) };
		Self(NonNull::new(ptr).expect("a live object"))
	}

	pub fn ptr(&self) -> *mut PyObject {
		self.0.as_ptr()
	}

	/// The reference, for a caller that steals it.
	pub fn into_ptr(self) -> *mut PyObject {
		let ptr = self.ptr();
		std::mem::forget(self);
		ptr
	}
}

impl Drop for Obj {
	fn drop(&mut self) {
		// SAFETY: a strong reference, dropped with the GIL held, as every `Obj` is.
		unsafe { Py_DecRef(self.ptr()) };
	}
}

/// Raise `exc` with `msg`.
pub fn raise<T>(exc: *mut PyObject, msg: &str) -> R<T> {
	// SAFETY: the GIL is held, and `exc` is an exception type.
	unsafe {
		let msg = PyUnicode_FromStringAndSize(msg.as_ptr().cast(), msg.len() as Py_ssize_t);
		if !msg.is_null() {
			PyErr_SetObject(exc, msg);
			Py_DecRef(msg);
		}
	}
	Err(Raised)
}

pub fn type_error<T>(msg: &str) -> R<T> {
	// SAFETY: a static exception type.
	raise(unsafe { PyExc_TypeError }, msg)
}

pub fn value_error<T>(msg: &str) -> R<T> {
	// SAFETY: a static exception type.
	raise(unsafe { PyExc_ValueError }, msg)
}

fn overflow<T>(msg: &str) -> R<T> {
	// SAFETY: a static exception type.
	raise(unsafe { PyExc_OverflowError }, msg)
}

/// Run an extension function's body, returning null once it raised.
pub fn wrap(f: impl FnOnce() -> R<Obj>) -> *mut PyObject {
	match f() {
		Ok(obj) => obj.into_ptr(),
		Err(Raised) => null_mut(),
	}
}

/// The `n` arguments of a fast call of `name`.
///
/// # Safety
/// `args` points at `nargs` live objects.
pub unsafe fn args<'a>(args: *mut *mut PyObject, nargs: Py_ssize_t, n: usize, name: &str) -> R<&'a [*mut PyObject]> {
	if nargs as usize != n {
		return type_error(&format!("{name}() takes {n} arguments ({nargs} given)"));
	}
	match n {
		0 => Ok(&[]),
		// SAFETY: the caller's contract.
		n => Ok(unsafe { std::slice::from_raw_parts(args, n) }),
	}
}

/// Call into the C ABI with the GIL released, so Rust may call Python back from any
/// thread while this one waits.
pub fn call<T>(f: impl FnOnce() -> T) -> T {
	// SAFETY: this thread holds the GIL, and touches no Python object until it retakes it.
	let state = unsafe { PyEval_SaveThread() };
	let result = f();
	// SAFETY: released just above.
	unsafe { PyEval_RestoreThread(state) };
	result
}

/// Run `f` calling into Python, from whatever thread Rust calls on, holding the GIL; an
/// exception it raises aborts with what it was, since Rust has nowhere to unwind it to.
pub fn upcall<T>(what: &str, f: impl FnOnce() -> R<T>) -> T {
	// SAFETY: any thread may take the GIL.
	let gil = unsafe { PyGILState_Ensure() };
	match f() {
		Ok(value) => {
			// SAFETY: taken above.
			unsafe { PyGILState_Release(gil) };
			value
		}
		Err(Raised) => {
			// SAFETY: the GIL is held, and an exception is set.
			unsafe { PyErr_Print() };
			rt::fail(what, "a call into Python raised")
		}
	}
}

/// A Python string interned once, such as a field's name.
pub struct Key(&'static CStr, AtomicPtr<PyObject>);

impl Key {
	pub const fn new(name: &'static CStr) -> Self {
		Self(name, AtomicPtr::new(null_mut()))
	}

	/// The string, which lives as long as the interpreter.
	pub fn get(&self) -> R<*mut PyObject> {
		let ptr = self.1.load(Ordering::Relaxed);
		if !ptr.is_null() {
			return Ok(ptr);
		}
		// SAFETY: the GIL is held.
		let ptr = unsafe { PyUnicode_InternFromString(self.0.as_ptr()) };
		if ptr.is_null() {
			return Err(Raised);
		}
		self.1.store(ptr, Ordering::Relaxed);
		Ok(ptr)
	}
}

/// What a call borrows from Python until it returns: the objects whose memory it reads,
/// the buffers it views, the handles it holds and those it moves out of, and what it
/// gives Rust, taken back if the call never happens.
#[derive(Default)]
pub struct Scope {
	keep: Vec<Obj>,
	views: Vec<Box<Py_buffer>>,
	arrays: Vec<Box<dyn std::any::Any>>,
	held: Vec<Obj>,
	moved: Vec<*mut Handle>,
	/// The objects Rust lends a callback, closed once it returns.
	lent: Vec<Obj>,
	/// Python implementations and waker nodes given to Rust, taken back unless the call
	/// happens.
	given: Vec<Obj>,
	nodes: Vec<rt::Waker>,
}

impl Scope {
	/// Keep `obj` until the call returns, returning it borrowed.
	pub fn keep(&mut self, obj: Obj) -> *mut PyObject {
		let ptr = obj.ptr();
		self.keep.push(obj);
		ptr
	}

	/// Note that the call happened: what it moved out of is closed, and what it was given
	/// is Rust's.
	pub fn done(&mut self) {
		for handle in self.moved.drain(..) {
			// SAFETY: a handle this scope holds, so live.
			unsafe { close(handle) };
		}
		for given in self.given.drain(..) {
			given.into_ptr();
		}
		self.nodes.clear();
	}

	/// The handles held, which a future keeps until it ends.
	pub fn take_held(&mut self) -> Vec<Obj> {
		std::mem::take(&mut self.held)
	}
}

impl Drop for Scope {
	fn drop(&mut self) {
		for waker in self.nodes.drain(..) {
			// SAFETY: a reference the call never took.
			unsafe { node_drop(waker.data) };
		}
		for lent in self.lent.drain(..) {
			// SAFETY: an object [`lent`] made.
			unsafe { close(lent.ptr().cast()) };
		}
		for view in &mut self.views {
			// SAFETY: a view `PyObject_GetBuffer` filled, released once.
			unsafe { PyBuffer_Release(&mut **view) };
		}
		for held in self.held.drain(..) {
			// SAFETY: a handle this scope holds.
			unsafe { release(held.ptr().cast()) };
		}
	}
}

/// The classes the generated Python registers, by the index the glue names each with:
/// the runtime's, then the API's.
static TYPES: Gil<Vec<Obj>> = Gil::new(Vec::new());

/// The plain enums' members, by class, with their discriminants.
static MEMBERS: Gil<Vec<Option<Vec<(i64, Obj)>>>> = Gil::new(Vec::new());

/// The runtime's classes, at the start of the registry: the awaitable a pending future
/// is, and `datetime.timedelta`.
pub const T_FUTURE: usize = 0;
const T_TIMEDELTA: usize = 1;

/// The class registered at `index`.
pub fn ty(index: usize) -> *mut PyObject {
	// SAFETY: the GIL is held, and the module registered its classes when it loaded.
	(unsafe { TYPES.get() })[index].ptr()
}

/// `_register(*classes)`: note the classes the glue makes values of.
unsafe extern "C" fn register(_: *mut PyObject, args: *mut *mut PyObject, nargs: Py_ssize_t) -> *mut PyObject {
	// SAFETY: Python's call.
	let args = unsafe { std::slice::from_raw_parts(args, nargs as usize) };
	// SAFETY: the GIL is held, and each argument a live class.
	unsafe {
		*TYPES.get() = args.iter().map(|&class| Obj::borrowed(class)).collect();
		*MEMBERS.get() = (0..args.len()).map(|_| None).collect();
	}
	none().map_or(null_mut(), Obj::into_ptr)
}

pub fn none() -> R<Obj> {
	// SAFETY: `None` is immortal; a reference to it is still counted on older versions.
	Ok(unsafe { Obj::borrowed(Py_None()) })
}

pub fn is_none(obj: *mut PyObject) -> bool {
	// SAFETY: comparing addresses.
	obj == unsafe { Py_None() }
}

/// Whether `obj` is an instance of the class registered at `index`.
pub fn is(obj: *mut PyObject, index: usize) -> bool {
	// SAFETY: the GIL is held, and `obj` is live.
	unsafe { PyObject_TypeCheck(obj, ty(index).cast()) != 0 }
}

/// A `TypeError` naming the class registered at `index` and what came instead.
fn expected<T>(obj: *mut PyObject, index: usize) -> R<T> {
	expected_named(obj, &type_name(ty(index)))
}

fn expected_named<T>(obj: *mut PyObject, want: &str) -> R<T> {
	// SAFETY: `obj` is live.
	let got = type_name(unsafe { Py_TYPE(obj) }.cast());
	type_error(&format!("expected {want}, got {got}"))
}

/// A class's name, for an error.
fn type_name(class: *mut PyObject) -> String {
	// SAFETY: the GIL is held, and `class` is a type.
	unsafe {
		let Ok(name) = Obj::new(PyType_GetName(class.cast())) else {
			PyErr_Clear();
			return "?".to_string();
		};
		let mut len = 0;
		let ptr = PyUnicode_AsUTF8AndSize(name.ptr(), &mut len);
		if ptr.is_null() {
			PyErr_Clear();
			return "?".to_string();
		}
		String::from_utf8_lossy(std::slice::from_raw_parts(ptr.cast(), len as usize)).into_owned()
	}
}

/// The attribute `key` of `obj`, kept for the call.
pub fn attr(obj: *mut PyObject, key: &Key, s: &mut Scope) -> R<*mut PyObject> {
	// SAFETY: the GIL is held, and `obj` is live.
	let value = unsafe { Obj::new(PyObject_GetAttr(obj, key.get()?)) }?;
	Ok(s.keep(value))
}

pub fn bool_in(obj: *mut PyObject) -> R<bool> {
	// SAFETY: comparing addresses.
	unsafe {
		if obj == Py_True() {
			Ok(true)
		} else if obj == Py_False() {
			Ok(false)
		} else {
			expected_named(obj, "bool")
		}
	}
}

pub fn bool_out(value: bool) -> R<Obj> {
	// SAFETY: the GIL is held.
	unsafe { Obj::new(PyBool_FromLong(value as _)) }
}

/// An integer of Rust's type `T`, from any `int` or object with `__index__` in its range.
pub fn int_in<T: TryFrom<i128>>(obj: *mut PyObject) -> R<T> {
	// SAFETY: the GIL is held, and `obj` is live.
	let value = unsafe {
		let index = Obj::new(PyNumber_Index(obj))?;
		let mut overflow = 0;
		let v = PyLong_AsLongLongAndOverflow(index.ptr(), &mut overflow);
		match overflow {
			0 if v == -1 && !PyErr_Occurred().is_null() => return Err(Raised),
			0 => v as i128,
			// Past `i64`: the `u64`s above it, or out of every range.
			_ => {
				let v = PyLong_AsUnsignedLongLong(index.ptr());
				if v == u64::MAX && !PyErr_Occurred().is_null() {
					PyErr_Clear();
					return overflow_int::<T>();
				}
				v as i128
			}
		}
	};
	T::try_from(value).or_else(|_| overflow_int::<T>())
}

fn overflow_int<T>() -> R<T> {
	let name = std::any::type_name::<T>();
	overflow(&format!("int out of range for Rust's {name}"))
}

pub fn int_out(value: i64) -> R<Obj> {
	// SAFETY: the GIL is held.
	unsafe { Obj::new(PyLong_FromLongLong(value)) }
}

pub fn uint_out(value: u64) -> R<Obj> {
	// SAFETY: the GIL is held.
	unsafe { Obj::new(PyLong_FromUnsignedLongLong(value)) }
}

pub fn float_in(obj: *mut PyObject) -> R<f64> {
	// SAFETY: the GIL is held, and `obj` is live.
	let value = unsafe { PyFloat_AsDouble(obj) };
	// SAFETY: as above.
	if value == -1.0 && !unsafe { PyErr_Occurred() }.is_null() {
		return Err(Raised);
	}
	Ok(value)
}

pub fn float_out(value: f64) -> R<Obj> {
	// SAFETY: the GIL is held.
	unsafe { Obj::new(PyFloat_FromDouble(value)) }
}

/// A `char`, from a `str` of one code point that is a Unicode scalar value.
pub fn char_in(obj: *mut PyObject) -> R<u32> {
	// SAFETY: the GIL is held, and `obj` is live.
	unsafe {
		if PyUnicode_Check(obj) == 0 || PyUnicode_GetLength(obj) != 1 {
			return expected_named(obj, "a str of one character");
		}
		let code = PyUnicode_ReadChar(obj, 0);
		match char::from_u32(code) {
			Some(_) => Ok(code),
			None => value_error("a surrogate is no Rust char"),
		}
	}
}

pub fn char_out(code: u32) -> R<Obj> {
	// SAFETY: the GIL is held.
	unsafe { Obj::new(PyUnicode_FromOrdinal(code as c_int)) }
}

/// A view of a `str`'s UTF-8, which Python caches in the string, so it lives as long as
/// `obj`: the caller keeps `obj` for the call.
pub fn str_in(obj: *mut PyObject) -> R<rt::Buf> {
	// SAFETY: the GIL is held, and `obj` is live.
	unsafe {
		if PyUnicode_Check(obj) == 0 {
			return expected_named(obj, "str");
		}
		let mut len = 0;
		let ptr = PyUnicode_AsUTF8AndSize(obj, &mut len);
		if ptr.is_null() {
			return Err(Raised);
		}
		Ok(rt::Buf {
			ptr: ptr.cast(),
			len: len as usize,
			owner: null_mut(),
		})
	}
}

/// A `str` of a buffer of UTF-8 Rust returned, freeing it.
pub fn str_out(buf: rt::Buf) -> R<Obj> {
	// SAFETY: a buffer Rust returned, freed once copied.
	unsafe {
		let str = Obj::new(PyUnicode_FromStringAndSize(buf.ptr.cast(), buf.len as Py_ssize_t));
		buf.free();
		str
	}
}

/// A view of any object with the buffer protocol, `bytes`, `bytearray`, or a
/// `memoryview` among them, held for the call: Rust reads it in place.
pub fn bytes_in(obj: *mut PyObject, s: &mut Scope) -> R<rt::Buf> {
	// SAFETY: `Py_buffer` is plain data, which the call fills.
	let mut view: Box<Py_buffer> = Box::new(unsafe { std::mem::zeroed() });
	// SAFETY: the GIL is held, and `obj` is live.
	if unsafe { PyObject_GetBuffer(obj, &mut *view, PyBUF_SIMPLE) } != 0 {
		return Err(Raised);
	}
	let buf = rt::Buf {
		ptr: view.buf.cast_const().cast(),
		len: view.len as usize,
		owner: null_mut(),
	};
	s.views.push(view);
	Ok(buf)
}

/// Bytes Rust returned, as a read-only `memoryview` that lends them without a copy until
/// it and every view of it are released.
pub fn bytes_out(buf: rt::Buf) -> R<Obj> {
	// A view Rust lent only for a call is copied into a buffer of its own.
	let buf = match buf.owner.is_null() {
		// SAFETY: a buffer Rust lent for the call.
		true => rt::Buf::owned(bytes::Bytes::copy_from_slice(unsafe { buf.bytes("python") })),
		false => buf,
	};
	// SAFETY: the GIL is held, and the owner type's slots are this module's.
	unsafe {
		let owner = PyType_GenericAlloc(BYTES.get().expect("loaded").ptr().cast(), 0);
		let Ok(owner) = Obj::new(owner) else {
			buf.free();
			return Err(Raised);
		};
		(*owner.ptr().cast::<Bytes>()).buf = buf;
		Obj::new(PyMemoryView_FromObject(owner.ptr()))
	}
}

/// Bytes Rust lends a callback for its call, copied into `bytes`.
pub fn lent_bytes(buf: rt::Buf) -> R<Obj> {
	// SAFETY: the GIL is held, and a buffer Rust lent for the call.
	unsafe {
		let bytes = buf.bytes("python");
		Obj::new(PyBytes_FromStringAndSize(bytes.as_ptr().cast(), bytes.len() as Py_ssize_t))
	}
}

/// A path's bytes, from a `str`, `bytes`, or `os.PathLike`, as `os.fsencode` encodes it,
/// so a name a Unix path holds that is not UTF-8 crosses as it came.
pub fn path_in(obj: *mut PyObject, s: &mut Scope) -> R<rt::Buf> {
	let mut encoded: *mut PyObject = null_mut();
	// SAFETY: the GIL is held, and `obj` is live; the converter writes a new `bytes`.
	unsafe {
		if PyUnicode_FSConverter(obj, (&raw mut encoded).cast()) == 0 {
			return Err(Raised);
		}
		let encoded = s.keep(Obj::new(encoded)?);
		let mut ptr: *mut c_char = null_mut();
		let mut len = 0;
		if PyBytes_AsStringAndSize(encoded, &mut ptr, &mut len) != 0 {
			return Err(Raised);
		}
		Ok(rt::Buf {
			ptr: ptr.cast_const().cast(),
			len: len as usize,
			owner: null_mut(),
		})
	}
}

/// A path Rust returned, as a `str` `os.fsdecode` decodes, freeing it.
pub fn path_out(buf: rt::Buf) -> R<Obj> {
	// SAFETY: a buffer Rust returned, freed once decoded.
	unsafe {
		let path = Obj::new(PyUnicode_DecodeFSDefaultAndSize(buf.ptr.cast(), buf.len as Py_ssize_t));
		buf.free();
		path
	}
}

static DAYS: Key = Key::new(c"days");
static SECONDS: Key = Key::new(c"seconds");
static MICROSECONDS: Key = Key::new(c"microseconds");
static MAX: Key = Key::new(c"max");

/// The days in `timedelta.max`, past which [`duration_out`] gives it.
const MAX_DAYS: u64 = 999_999_999;

/// A `Duration`, from a `datetime.timedelta` that is not negative: `timedelta.max`, which
/// [`duration_out`] gives for any longer one, is `Duration::MAX`.
pub fn duration_in(obj: *mut PyObject) -> R<rt::Duration> {
	if !is(obj, T_TIMEDELTA) {
		return expected(obj, T_TIMEDELTA);
	}
	let mut s = Scope::default();
	let days: u64 = match int_in::<i64>(attr(obj, &DAYS, &mut s)?)? {
		days if days < 0 => return value_error("a negative timedelta has no Rust Duration"),
		days => days as u64,
	};
	let seconds: u64 = int_in(attr(obj, &SECONDS, &mut s)?)?;
	let micros: u32 = int_in(attr(obj, &MICROSECONDS, &mut s)?)?;
	if (days, seconds, micros) == (MAX_DAYS, 86_399, 999_999) {
		return Ok(rt::Duration::lower(std::time::Duration::MAX));
	}
	Ok(rt::Duration {
		secs: days * 86_400 + seconds,
		nanos: micros * 1_000,
	})
}

/// A `datetime.timedelta`, which holds whole microseconds up to 999,999,999 days: Rust's
/// nanoseconds are truncated, and a longer one is `timedelta.max`, as Rust's
/// `Duration::MAX` stands for forever, which [`duration_in`] gives back.
pub fn duration_out(duration: rt::Duration) -> R<Obj> {
	let class = ty(T_TIMEDELTA);
	// SAFETY: the GIL is held.
	unsafe {
		if duration.secs / 86_400 > MAX_DAYS {
			return Obj::new(PyObject_GetAttr(class, MAX.get()?));
		}
		let args = tuple(vec![
			int_out(0),
			uint_out(duration.secs),
			uint_out(u64::from(duration.nanos / 1_000)),
		])?;
		Obj::new(PyObject_CallObject(class, args.ptr()))
	}
}

/// Which addresses a socket address type holds.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Family {
	Any,
	V4,
	V6,
}

/// A socket address, from a tuple as Python's `socket` module writes one: `(host, port)`,
/// or for IPv6, `(host, port, flowinfo, scope_id)`, its host a numeric address.
pub fn addr_in(obj: *mut PyObject, family: Family) -> R<rt::SocketAddr> {
	// SAFETY: the GIL is held, and `obj` is live.
	let len = unsafe {
		if PyTuple_Check(obj) == 0 {
			return expected_named(obj, "a (host, port) tuple");
		}
		PyTuple_Size(obj)
	};
	if len != 2 && len != 4 {
		return value_error("a socket address is (host, port) or (host, port, flowinfo, scope_id)");
	}
	// SAFETY: in range.
	let item = |n| unsafe { PyTuple_GetItem(obj, n) };
	let host = str_in(item(0))?;
	// SAFETY: a view of the host's UTF-8, which the tuple keeps.
	let host = unsafe { host.str("python") };
	let Ok(ip) = host.parse::<std::net::IpAddr>() else {
		return value_error(&format!("{host:?} is no numeric IP address"));
	};
	let port: u16 = int_in(item(1))?;
	let (flowinfo, scope_id) = match len {
		4 => (int_in::<u32>(item(2))?, int_in::<u32>(item(3))?),
		_ => (0, 0),
	};
	let addr = match ip {
		std::net::IpAddr::V4(_) if len == 4 => return value_error("an IPv4 address takes no flowinfo or scope_id"),
		std::net::IpAddr::V4(ip) => std::net::SocketAddr::from((ip, port)),
		std::net::IpAddr::V6(ip) => std::net::SocketAddrV6::new(ip, port, flowinfo, scope_id).into(),
	};
	match (family, addr) {
		(Family::V4, std::net::SocketAddr::V6(_)) => value_error("an IPv6 address has no Rust SocketAddrV4"),
		(Family::V6, std::net::SocketAddr::V4(_)) => value_error("an IPv4 address has no Rust SocketAddrV6"),
		_ => Ok(rt::SocketAddr::lower(addr)),
	}
}

/// A socket address as Python's `socket` module writes one.
pub fn addr_out(addr: rt::SocketAddr) -> R<Obj> {
	let text = |ip: &dyn std::fmt::Display| {
		let text = ip.to_string();
		// SAFETY: the GIL is held.
		unsafe { Obj::new(PyUnicode_FromStringAndSize(text.as_ptr().cast(), text.len() as Py_ssize_t)) }
	};
	match addr.lift("python") {
		std::net::SocketAddr::V4(v4) => tuple(vec![text(v4.ip()), uint_out(v4.port().into())]),
		std::net::SocketAddr::V6(v6) => tuple(vec![
			text(v6.ip()),
			uint_out(v6.port().into()),
			uint_out(v6.flowinfo().into()),
			uint_out(v6.scope_id().into()),
		]),
	}
}

static START: Key = Key::new(c"start");
static STOP: Key = Key::new(c"stop");
static STEP: Key = Key::new(c"step");

/// A `Range`, from a `range` of step 1.
pub fn range_in<T: TryFrom<i128>>(obj: *mut PyObject) -> R<rt::Range<T>> {
	// SAFETY: the GIL is held, and `obj` is live.
	if unsafe { PyObject_TypeCheck(obj, &raw mut PyRange_Type) } == 0 {
		return expected_named(obj, "range");
	}
	let mut s = Scope::default();
	if int_in::<i64>(attr(obj, &STEP, &mut s)?)? != 1 {
		return value_error("a range of a step other than 1 has no Rust Range");
	}
	Ok(rt::Range {
		start: int_in(attr(obj, &START, &mut s)?)?,
		end: int_in(attr(obj, &STOP, &mut s)?)?,
	})
}

pub fn range_out(start: R<Obj>, end: R<Obj>) -> R<Obj> {
	let args = tuple(vec![start, end])?;
	// SAFETY: the GIL is held.
	unsafe { Obj::new(PyObject_CallObject((&raw mut PyRange_Type).cast(), args.ptr())) }
}

/// A tuple of `items`, each made before any error is returned, so each frees what Rust
/// handed over.
pub fn tuple(items: Vec<R<Obj>>) -> R<Obj> {
	// SAFETY: the GIL is held.
	unsafe {
		let tuple = Obj::new(PyTuple_New(items.len() as Py_ssize_t));
		let mut raised = false;
		for (n, item) in items.into_iter().enumerate() {
			match (&tuple, item) {
				(Ok(tuple), Ok(item)) => {
					PyTuple_SetItem(tuple.ptr(), n as Py_ssize_t, item.into_ptr());
				}
				(_, Err(Raised)) => raised = true,
				(Err(Raised), Ok(_)) => {}
			}
		}
		match raised {
			true => Err(Raised),
			false => tuple,
		}
	}
}

/// The item `n` of a tuple of `len`, or a `TypeError`.
pub fn tuple_item(obj: *mut PyObject, n: usize, len: usize) -> R<*mut PyObject> {
	// SAFETY: the GIL is held, and `obj` is live.
	unsafe {
		if PyTuple_Check(obj) == 0 || PyTuple_Size(obj) != len as Py_ssize_t {
			return expected_named(obj, &format!("a tuple of {len}"));
		}
		Ok(PyTuple_GetItem(obj, n as Py_ssize_t))
	}
}

/// A `Vec` of the items any iterable yields, each read by `each`, held for the call.
pub fn list_in<T: Copy + 'static>(
	obj: *mut PyObject,
	s: &mut Scope,
	mut each: impl FnMut(*mut PyObject, &mut Scope) -> R<T>,
) -> R<rt::List<T>> {
	// SAFETY: the GIL is held, and `obj` is live.
	unsafe {
		// A str iterates its characters, which no Rust list of strings means.
		if PyUnicode_Check(obj) != 0 {
			return expected_named(obj, "a list");
		}
		let iter = Obj::new(PyObject_GetIter(obj))?;
		let mut items = Vec::new();
		loop {
			let item = PyIter_Next(iter.ptr());
			if item.is_null() {
				if !PyErr_Occurred().is_null() {
					return Err(Raised);
				}
				break;
			}
			let item = s.keep(Obj::new(item)?);
			items.push(each(item, s)?);
		}
		let items = Box::new(items);
		let list = rt::List {
			ptr: items.as_ptr(),
			len: items.len(),
			owner: null_mut(),
		};
		s.arrays.push(items);
		Ok(list)
	}
}

/// A `list` of the elements of an array Rust returned, each made by `each`, which frees
/// what it holds, before the array is freed.
pub fn list_out<T: Copy + 'static>(list: rt::List<T>, mut each: impl FnMut(T) -> R<Obj>) -> R<Obj> {
	// SAFETY: an array Rust returned, whose elements `each` takes.
	let items: Vec<R<Obj>> = unsafe { list.items("python") }.iter().map(|&item| each(item)).collect();
	// SAFETY: as above, with its elements taken.
	unsafe { rt::List { len: 0, ..list }.free("python", |_| {}) };
	// SAFETY: the GIL is held.
	unsafe {
		let out = Obj::new(PyList_New(items.len() as Py_ssize_t));
		let mut raised = false;
		for (n, item) in items.into_iter().enumerate() {
			match (&out, item) {
				(Ok(out), Ok(item)) => {
					PyList_SetItem(out.ptr(), n as Py_ssize_t, item.into_ptr());
				}
				(_, Err(Raised)) => raised = true,
				(Err(Raised), Ok(_)) => {}
			}
		}
		match raised {
			true => Err(Raised),
			false => out,
		}
	}
}

/// An instance of the class registered at `index` with `fields` set, each made before
/// any error is returned, so each frees what Rust handed over. Made as `object.__new__`
/// makes one and set as `object.__setattr__` sets, so no `__init__` runs.
pub fn record(index: usize, fields: Vec<(&Key, R<Obj>)>) -> R<Obj> {
	let class = ty(index);
	// SAFETY: the GIL is held, and `class` a class of the generated Python.
	unsafe {
		let alloc: allocfunc = std::mem::transmute(PyType_GetSlot(class.cast(), Py_tp_alloc));
		let obj = Obj::new(alloc(class.cast(), 0));
		let mut raised = false;
		for (key, value) in fields {
			match (&obj, value, key.get()) {
				(Ok(obj), Ok(value), Ok(key)) => {
					raised |= PyObject_GenericSetAttr(obj.ptr(), key, value.ptr()) != 0;
				}
				_ => raised = true,
			}
		}
		match raised {
			true => Err(Raised),
			false => obj,
		}
	}
}

/// The field `key` of a record or variant, after checking `obj` is of the class
/// registered at `index`.
pub fn field(obj: *mut PyObject, index: usize, key: &Key, s: &mut Scope) -> R<*mut PyObject> {
	match is(obj, index) {
		true => attr(obj, key, s),
		false => expected(obj, index),
	}
}

/// Check `obj` is of the class registered at `index`.
pub fn check(obj: *mut PyObject, index: usize) -> R<()> {
	match is(obj, index) {
		true => Ok(()),
		false => expected(obj, index),
	}
}

static VALUE: Key = Key::new(c"value");

/// The members of the plain enum registered at `index`, with their discriminants.
fn members(index: usize) -> R<&'static [(i64, Obj)]> {
	// SAFETY: the GIL is held.
	let table = unsafe { MEMBERS.get() };
	if table[index].is_none() {
		let mut members = Vec::new();
		// SAFETY: the GIL is held, and an `Enum` class iterates its members.
		unsafe {
			let iter = Obj::new(PyObject_GetIter(ty(index)))?;
			loop {
				let member = PyIter_Next(iter.ptr());
				if member.is_null() {
					if !PyErr_Occurred().is_null() {
						return Err(Raised);
					}
					break;
				}
				let member = Obj::new(member)?;
				let value = Obj::new(PyObject_GetAttr(member.ptr(), VALUE.get()?))?;
				members.push((int_in::<i64>(value.ptr())?, member));
			}
		}
		table[index] = Some(members);
	}
	Ok(table[index].as_deref().expect("set above"))
}

/// The member of the plain enum registered at `index` whose discriminant is `value`.
pub fn member(index: usize, value: i64) -> R<Obj> {
	match members(index)?.iter().find(|(v, _)| *v == value) {
		// SAFETY: a live member.
		Some((_, member)) => Ok(unsafe { Obj::borrowed(member.ptr()) }),
		None => rt::fail("python", "an enum's discriminant is invalid"),
	}
}

/// The discriminant of `obj`, a member of the plain enum registered at `index`.
pub fn member_in(obj: *mut PyObject, index: usize) -> R<i32> {
	match members(index)?.iter().find(|(_, m)| m.ptr() == obj) {
		Some((value, _)) => Ok(*value as i32),
		None => expected(obj, index),
	}
}

/// Raise the exception class registered at `index`, holding `error`.
pub fn raise_error<T>(index: usize, error: R<Obj>) -> R<T> {
	let args = tuple(vec![error])?;
	// SAFETY: the GIL is held, and the class an exception's.
	unsafe {
		let exc = Obj::new(PyObject_CallObject(ty(index), args.ptr()))?;
		PyErr_SetObject(ty(index), exc.ptr());
	}
	Err(Raised)
}

/// Call `callable` with `args`, each made before any error is returned.
pub fn call_py(callable: *mut PyObject, args: Vec<R<Obj>>) -> R<Obj> {
	let args = tuple(args)?;
	// SAFETY: the GIL is held, and `callable` is live.
	unsafe { Obj::new(PyObject_CallObject(callable, args.ptr())) }
}

/// Call the method `key` of `obj` with `args`.
pub fn call_method(obj: *mut PyObject, key: &Key, args: Vec<R<Obj>>) -> R<Obj> {
	// SAFETY: the GIL is held, and `obj` is live.
	let method = unsafe { Obj::new(PyObject_GetAttr(obj, key.get()?)) }?;
	call_py(method.ptr(), args)
}

/// A Python implementation of a trait, an instance of the abstract class registered at
/// `index`, given to Rust: a reference Rust drops through [`free_callback`], or the scope
/// drops if the call never happens.
pub fn callback(obj: *mut PyObject, index: usize, s: &mut Scope) -> R<*mut c_void> {
	check(obj, index)?;
	// SAFETY: the GIL is held, and `obj` is live.
	let given = unsafe { Obj::borrowed(obj) };
	let ptr = given.ptr();
	s.given.push(given);
	Ok(ptr.cast())
}

/// A callable Rust calls only during the call, as a closure's data: the caller's reference
/// outlives it.
pub fn closure(obj: *mut PyObject) -> R<*mut c_void> {
	// SAFETY: the GIL is held, and `obj` is live.
	match unsafe { PyCallable_Check(obj) } {
		0 => expected_named(obj, "a callable"),
		_ => Ok(obj.cast()),
	}
}

static CLOSE: Key = Key::new(c"close");

/// Let go of a Python implementation of a trait, from any thread: closed first if it has
/// a `close` method.
///
/// # Safety
/// `data` is a reference [`callback`] gave Rust, freed once.
pub unsafe extern "C" fn free_callback(data: *mut c_void) {
	upcall("free", || {
		// SAFETY: the reference Rust held.
		let obj = unsafe { Obj::new(data.cast()) }?;
		// SAFETY: the GIL is held.
		if unsafe { PyObject_HasAttr(obj.ptr(), CLOSE.get()?) } == 1 {
			call_method(obj.ptr(), &CLOSE, Vec::new())?;
		}
		Ok(())
	})
}

/// An object owning a handle Rust returned, or one Rust lends for a call: the instance
/// of a class of the generated Python, which subclasses `Object`.
#[repr(C)]
pub struct Handle {
	ob_base: PyObject,
	raw: *const c_void,
	/// What frees the handle: none for one Rust lends, or once freed.
	free: Option<unsafe extern "C" fn(*const c_void)>,
	/// The calls holding it, each of which it outlives.
	holds: usize,
	closed: bool,
}

/// A new object of the class registered at `index` owning `raw`, which `free` frees.
pub fn object(raw: *const c_void, index: usize, free: unsafe extern "C" fn(*const c_void)) -> R<Obj> {
	handle(raw, index, Some(free))
}

/// [`object`], or `None` for a null handle.
pub fn object_opt(raw: *const c_void, index: usize, free: unsafe extern "C" fn(*const c_void)) -> R<Obj> {
	match raw.is_null() {
		true => none(),
		false => object(raw, index, free),
	}
}

/// An object of the class registered at `index` for a handle Rust lends a callback,
/// closed once the scope `s` of the call ends.
pub fn lent(raw: *const c_void, index: usize, s: &mut Scope) -> R<Obj> {
	let obj = handle(raw, index, None)?;
	// SAFETY: the GIL is held, and `obj` is live.
	s.lent.push(unsafe { Obj::borrowed(obj.ptr()) });
	Ok(obj)
}

fn handle(raw: *const c_void, index: usize, free: Option<unsafe extern "C" fn(*const c_void)>) -> R<Obj> {
	let class = ty(index);
	// SAFETY: the GIL is held, and `class` subclasses `Object` with no fields of its own.
	unsafe {
		let obj = PyType_GenericAlloc(class.cast(), 0);
		let Ok(obj) = Obj::new(obj) else {
			if let Some(free) = free {
				free(raw);
			}
			return Err(Raised);
		};
		let handle = obj.ptr().cast::<Handle>();
		(*handle).raw = raw;
		(*handle).free = free;
		(*handle).holds = 0;
		(*handle).closed = false;
		Ok(obj)
	}
}

/// The handle `obj` owns, an object of the class registered at `index`, held for the
/// call; `moved` when the call moves its value out, after which `obj` is closed.
pub fn hold(obj: *mut PyObject, index: usize, s: &mut Scope, moved: bool) -> R<*const c_void> {
	check(obj, index)?;
	let handle = obj.cast::<Handle>();
	// SAFETY: an object of a class that subclasses `Object`, and the GIL is held.
	unsafe {
		if (*handle).closed {
			return value_error(&format!("{} is closed", type_name(ty(index))));
		}
		(*handle).holds += 1;
		s.held.push(Obj::borrowed(obj));
		if moved {
			s.moved.push(handle);
		}
		Ok((*handle).raw)
	}
}

/// [`hold`], or a null handle for `None`.
pub fn hold_opt(obj: *mut PyObject, index: usize, s: &mut Scope, moved: bool) -> R<*const c_void> {
	match is_none(obj) {
		true => Ok(null()),
		false => hold(obj, index, s, moved),
	}
}

/// Let go of a call's hold, freeing the handle if it was closed meanwhile.
///
/// # Safety
/// `handle` is live, and held.
unsafe fn release(handle: *mut Handle) {
	// SAFETY: the caller's contract.
	unsafe {
		(*handle).holds -= 1;
		if (*handle).closed && (*handle).holds == 0 {
			free_now(handle);
		}
	}
}

/// Close a handle: freed now, or once the last call holding it returns.
///
/// # Safety
/// `handle` is live.
pub unsafe fn close(handle: *mut Handle) {
	// SAFETY: the caller's contract.
	unsafe {
		if (*handle).closed {
			return;
		}
		(*handle).closed = true;
		if (*handle).holds == 0 {
			free_now(handle);
		}
	}
}

/// # Safety
/// `handle` is live and held by no call.
unsafe fn free_now(handle: *mut Handle) {
	// SAFETY: the caller's contract.
	unsafe {
		if let Some(free) = (*handle).free.take() {
			let raw = std::mem::replace(&mut (*handle).raw, null());
			call(|| free(raw));
		}
	}
}

unsafe extern "C" fn handle_close(obj: *mut PyObject, _: *mut PyObject) -> *mut PyObject {
	// SAFETY: Python calls it on an `Object`.
	unsafe { close(obj.cast()) };
	none().map_or(null_mut(), Obj::into_ptr)
}

unsafe extern "C" fn handle_enter(obj: *mut PyObject, _: *mut PyObject) -> *mut PyObject {
	// SAFETY: Python calls it on a live object.
	unsafe { Obj::borrowed(obj) }.into_ptr()
}

unsafe extern "C" fn handle_exit(obj: *mut PyObject, _: *mut *mut PyObject, _: Py_ssize_t) -> *mut PyObject {
	// SAFETY: Python calls it on an `Object`.
	unsafe { close(obj.cast()) };
	none().map_or(null_mut(), Obj::into_ptr)
}

unsafe extern "C" fn handle_dealloc(obj: *mut PyObject) {
	// SAFETY: Python deallocates an `Object` once, holding the GIL; nothing holds it.
	unsafe {
		free_now(obj.cast());
		dealloc(obj);
	}
}

/// Free an instance of a heap type, and the reference to its type it held.
///
/// # Safety
/// `obj` is being deallocated, holding the GIL.
unsafe fn dealloc(obj: *mut PyObject) {
	// SAFETY: the caller's contract.
	unsafe {
		let class = Py_TYPE(obj);
		let free: freefunc = std::mem::transmute(PyType_GetSlot(class, Py_tp_free));
		free(obj.cast());
		Py_DecRef(class.cast());
	}
}

/// The owner of bytes Rust returned, which a `memoryview` reads in place.
#[repr(C)]
struct Bytes {
	ob_base: PyObject,
	buf: rt::Buf,
}

static BYTES: std::sync::OnceLock<Static> = std::sync::OnceLock::new();

/// A reference kept for the interpreter's life.
struct Static(Obj);

// SAFETY: touched only with the GIL held.
unsafe impl Send for Static {}
// SAFETY: as above.
unsafe impl Sync for Static {}

impl Static {
	fn ptr(&self) -> *mut PyObject {
		self.0.ptr()
	}
}

unsafe extern "C" fn bytes_getbuffer(obj: *mut PyObject, view: *mut Py_buffer, flags: c_int) -> c_int {
	// SAFETY: Python asks a live `Bytes` for a view.
	unsafe {
		let buf = (*obj.cast::<Bytes>()).buf;
		PyBuffer_FillInfo(view, obj, buf.ptr.cast_mut().cast(), buf.len as Py_ssize_t, 1, flags)
	}
}

unsafe extern "C" fn bytes_dealloc(obj: *mut PyObject) {
	// SAFETY: Python deallocates a `Bytes` once no view of it is left.
	unsafe {
		(*obj.cast::<Bytes>()).buf.free();
		dealloc(obj);
	}
}

/// What a wake does: queue a token for the event loop, and write a byte it reads, once
/// until it takes them.
pub struct Queue {
	tokens: Mutex<Vec<i64>>,
	signaled: AtomicBool,
	tx: Socket,
	rx: Socket,
}

#[cfg(unix)]
type Socket = std::os::unix::net::UnixStream;
#[cfg(windows)]
type Socket = std::net::TcpStream;

/// A connected pair of sockets that never block.
#[cfg(unix)]
fn socket_pair() -> std::io::Result<(Socket, Socket)> {
	let (tx, rx) = Socket::pair()?;
	tx.set_nonblocking(true)?;
	rx.set_nonblocking(true)?;
	Ok((tx, rx))
}

/// A connected pair of sockets that never block, over loopback TCP, as Python's own
/// `socket.socketpair` makes one on Windows.
#[cfg(windows)]
fn socket_pair() -> std::io::Result<(Socket, Socket)> {
	let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
	let tx = Socket::connect(listener.local_addr()?)?;
	let (rx, _) = listener.accept()?;
	tx.set_nodelay(true)?;
	tx.set_nonblocking(true)?;
	rx.set_nonblocking(true)?;
	Ok((tx, rx))
}

impl Queue {
	fn push(&self, token: i64) {
		self.tokens.lock().unwrap_or_else(|e| e.into_inner()).push(token);
		if !self.signaled.swap(true, Ordering::SeqCst) {
			// A full socket already signals.
			let _ = (&self.tx).write(&[1]);
		}
	}

	/// The tokens queued, each once, in the order first queued: drained, then the signal
	/// reset, then the queue taken, so a token queued after is signaled again.
	fn take(&self) -> Vec<i64> {
		let mut sink = [0u8; 64];
		while matches!((&self.rx).read(&mut sink), Ok(n) if n > 0) {}
		self.signaled.store(false, Ordering::SeqCst);
		let tokens = std::mem::take(&mut *self.tokens.lock().unwrap_or_else(|e| e.into_inner()));
		let mut seen = std::collections::HashSet::new();
		tokens.into_iter().filter(|t| seen.insert(*t)).collect()
	}
}

/// What a waker Rust holds wakes: a future polled on an event loop, or a Python
/// `Waker`. A token names it to the loop; a `Waker`'s negative token says it is gone.
pub struct Node {
	state: AtomicU8,
	token: i64,
	queue: Arc<Queue>,
	/// Whether it is a `Waker`'s, whose every wake reaches the loop.
	waker: bool,
}

const IDLE: u8 = 0;
const POLLING: u8 = 1;
const PARKED: u8 = 2;
const WOKEN: u8 = 3;

static TOKENS: AtomicI64 = AtomicI64::new(1);

impl Node {
	fn new(queue: Arc<Queue>, waker: bool) -> Arc<Self> {
		Arc::new(Self {
			state: AtomicU8::new(IDLE),
			token: TOKENS.fetch_add(1, Ordering::Relaxed),
			queue,
			waker,
		})
	}

	/// A C waker holding a new reference to the node.
	fn give(self: &Arc<Self>) -> rt::Waker {
		rt::Waker {
			wake: Some(node_wake),
			drop: Some(node_drop),
			data: Arc::into_raw(self.clone()).cast_mut().cast(),
		}
	}

	/// The C waker [`Self::give`] gave, again: the C ABI reuses the one it holds.
	fn lend(self: &Arc<Self>) -> rt::Waker {
		rt::Waker {
			wake: Some(node_wake),
			drop: Some(node_drop),
			data: Arc::as_ptr(self).cast_mut().cast(),
		}
	}
}

impl Drop for Node {
	fn drop(&mut self) {
		if self.waker {
			self.queue.push(-self.token);
		}
	}
}

/// A wake, from any thread: a future's is latched while it polls, and queued once parked.
unsafe extern "C" fn node_wake(data: *mut c_void) {
	// SAFETY: Rust holds a reference to the node while it may wake it.
	let node = unsafe { &*data.cast::<Node>() };
	match node.waker {
		true => node.queue.push(node.token),
		false => {
			if node.state.swap(WOKEN, Ordering::AcqRel) == PARKED {
				node.queue.push(node.token);
			}
		}
	}
}

unsafe extern "C" fn node_drop(data: *mut c_void) {
	// SAFETY: a reference `give` made, dropped once.
	drop(unsafe { Arc::from_raw(data.cast::<Node>()) });
}

/// `Queue`: the queue of one event loop, with its socket's file descriptor to read.
#[repr(C)]
struct QueueObj {
	ob_base: PyObject,
	queue: Option<Arc<Queue>>,
}

unsafe extern "C" fn queue_new(class: *mut PyTypeObject, _: *mut PyObject, _: *mut PyObject) -> *mut PyObject {
	let (tx, rx) = match socket_pair() {
		Ok(pair) => pair,
		Err(err) => {
			// SAFETY: a static exception type.
			let _ = raise::<()>(unsafe { PyExc_OSError }, &err.to_string());
			return null_mut();
		}
	};
	// SAFETY: the GIL is held, and `class` is `Queue`.
	unsafe {
		let obj = PyType_GenericAlloc(class, 0);
		if !obj.is_null() {
			std::ptr::write(
				&raw mut (*obj.cast::<QueueObj>()).queue,
				Some(Arc::new(Queue {
					tokens: Mutex::new(Vec::new()),
					signaled: AtomicBool::new(false),
					tx,
					rx,
				})),
			);
		}
		obj
	}
}

unsafe extern "C" fn queue_dealloc(obj: *mut PyObject) {
	// SAFETY: Python deallocates a `Queue` once.
	unsafe {
		drop((*obj.cast::<QueueObj>()).queue.take());
		dealloc(obj);
	}
}

/// The queue a `Queue` holds.
fn queue_of(obj: *mut PyObject) -> R<Arc<Queue>> {
	if !is_type(obj, &QUEUE) {
		return expected_named(obj, "Queue");
	}
	// SAFETY: a `Queue`, which holds its queue until deallocated.
	Ok(unsafe { (*obj.cast::<QueueObj>()).queue.clone() }.expect("a live queue"))
}

unsafe extern "C" fn queue_fileno(obj: *mut PyObject, _: *mut PyObject) -> *mut PyObject {
	// SAFETY: Python calls it on a `Queue`.
	let queue = unsafe { (*obj.cast::<QueueObj>()).queue.as_ref() }.expect("a live queue");
	#[cfg(unix)]
	let fd = std::os::fd::AsRawFd::as_raw_fd(&queue.rx) as i64;
	#[cfg(windows)]
	let fd = std::os::windows::io::AsRawSocket::as_raw_socket(&queue.rx) as i64;
	int_out(fd).map_or(null_mut(), Obj::into_ptr)
}

unsafe extern "C" fn queue_take(obj: *mut PyObject, _: *mut PyObject) -> *mut PyObject {
	// SAFETY: Python calls it on a `Queue`.
	let queue = unsafe { (*obj.cast::<QueueObj>()).queue.as_ref() }.expect("a live queue");
	let tokens = queue.take();
	wrap(|| {
		let list = rt::List::owned(tokens);
		list_out(list, int_out)
	})
}

/// `Node`: a Python `Waker`'s node, made on its loop's queue.
#[repr(C)]
struct NodeObj {
	ob_base: PyObject,
	node: Option<Arc<Node>>,
}

unsafe extern "C" fn node_new(class: *mut PyTypeObject, args: *mut PyObject, _: *mut PyObject) -> *mut PyObject {
	wrap(|| {
		let queue = queue_of(tuple_item(args, 0, 1)?)?;
		// SAFETY: the GIL is held, and `class` is `Node`.
		unsafe {
			let obj = Obj::new(PyType_GenericAlloc(class, 0))?;
			std::ptr::write(&raw mut (*obj.ptr().cast::<NodeObj>()).node, Some(Node::new(queue, true)));
			Ok(obj)
		}
	})
}

unsafe extern "C" fn node_obj_dealloc(obj: *mut PyObject) {
	// SAFETY: Python deallocates a `Node` once.
	unsafe {
		drop((*obj.cast::<NodeObj>()).node.take());
		dealloc(obj);
	}
}

unsafe extern "C" fn node_token(obj: *mut PyObject, _: *mut PyObject) -> *mut PyObject {
	// SAFETY: Python calls it on a `Node`.
	let node = unsafe { (*obj.cast::<NodeObj>()).node.as_ref() }.expect("a live node");
	int_out(node.token).map_or(null_mut(), Obj::into_ptr)
}

static NODE_ATTR: Key = Key::new(c"_node");

/// A Python `Waker` passed by value: Rust holds a new reference to its node.
pub fn waker_in(obj: *mut PyObject, s: &mut Scope) -> R<rt::Waker> {
	let node = attr(obj, &NODE_ATTR, s)?;
	if !is_type(node, &NODE) {
		return expected_named(obj, "Waker");
	}
	// SAFETY: a `Node`, which holds its node until deallocated.
	let node = unsafe { (*node.cast::<NodeObj>()).node.clone() }.expect("a live node");
	let waker = node.give();
	s.nodes.push(waker);
	Ok(waker)
}

/// Poll a future once with a waker, returning its value once it finishes.
pub type Poll = unsafe fn(*mut c_void, rt::Waker) -> R<Option<Obj>>;

/// `Future`: a C ABI future its first poll left pending, which `__await__` polls on its
/// loop's queue until it finishes, and frees once it does or is closed, cancelling it.
#[repr(C)]
pub struct Future {
	ob_base: PyObject,
	raw: *mut c_void,
	poll: Option<Poll>,
	free: Option<unsafe fn(*mut c_void)>,
	node: Option<Arc<Node>>,
	token: i64,
	/// Whether an `await` took it, after which another raises, as awaiting a coroutine
	/// twice does.
	claimed: bool,
	/// The handles the call holds until the future ends.
	held: Vec<Obj>,
}

static PENDING: std::sync::OnceLock<Static> = std::sync::OnceLock::new();

/// A future the call `raw` made, polled at once unless `eager`, since a function that is
/// not `async` ran at the call and is polled when awaited: its value if ready, or else the
/// awaitable that polls it again, holding what the call held.
///
/// # Safety
/// `raw` is a live future that `poll` and `free` take.
pub unsafe fn start(raw: *mut c_void, poll: Poll, free: unsafe fn(*mut c_void), s: &mut Scope, eager: bool) -> R<Obj> {
	if !eager {
		let none = rt::Waker {
			wake: None,
			drop: None,
			data: null_mut(),
		};
		// SAFETY: the caller's contract.
		match unsafe { poll(raw, none) } {
			Ok(None) => {}
			done => {
				// SAFETY: as above, and finished or failed.
				call(|| unsafe { free(raw) });
				return done.map(|value| value.expect("ready"));
			}
		}
	}
	// SAFETY: the GIL is held, and the registered class subclasses `Future`.
	unsafe {
		let obj = PyType_GenericAlloc(ty(T_FUTURE).cast(), 0);
		let Ok(obj) = Obj::new(obj) else {
			call(|| free(raw));
			return Err(Raised);
		};
		// The allocation is zeroed, which is `None` for each option.
		let future = obj.ptr().cast::<Future>();
		(*future).raw = raw;
		(*future).poll = Some(poll);
		(*future).free = Some(free);
		(*future).token = TOKENS.fetch_add(1, Ordering::Relaxed);
		std::ptr::write(&raw mut (*future).held, s.take_held());
		Ok(obj)
	}
}

/// Free a future's C future, cancelling it unless it finished, and let go of its holds.
///
/// # Safety
/// `future` is live.
unsafe fn future_close(future: *mut Future) {
	// SAFETY: the caller's contract.
	unsafe {
		if let Some(free) = (*future).free.take() {
			let raw = std::mem::replace(&mut (*future).raw, null_mut());
			call(|| free(raw));
		}
		(*future).poll = None;
		(*future).node = None;
		for held in std::mem::take(&mut (*future).held) {
			release(held.ptr().cast());
		}
	}
}

/// `poll(queue)`: poll on the queue's loop, again at once if woken while polling: the
/// value, or `PENDING` once parked, after which a wake queues the future's token.
unsafe extern "C" fn future_poll(obj: *mut PyObject, args: *mut *mut PyObject, nargs: Py_ssize_t) -> *mut PyObject {
	let future = obj.cast::<Future>();
	wrap(|| {
		// SAFETY: Python's call.
		let args = unsafe { self::args(args, nargs, 1, "poll") }?;
		// SAFETY: Python calls it on a `Future`, holding the GIL.
		let this = unsafe { &mut *future };
		let Some(poll) = this.poll else {
			return value_error("the future ended");
		};
		let node = match &this.node {
			Some(node) => node.clone(),
			None => {
				let node = Arc::new(Node {
					state: AtomicU8::new(IDLE),
					token: this.token,
					queue: queue_of(args[0])?,
					waker: false,
				});
				this.node = Some(node.clone());
				node
			}
		};
		loop {
			let first = node.state.swap(POLLING, Ordering::AcqRel) == IDLE;
			// The first poll with the node gives the C ABI a reference, which it keeps
			// while it reuses the waker.
			let waker = match first {
				true => node.give(),
				false => node.lend(),
			};
			// SAFETY: a live future, which `poll` takes.
			let done = unsafe { poll(this.raw, waker) };
			match done {
				Ok(None) => {}
				done => {
					// SAFETY: a live future, finished or failed.
					unsafe { future_close(future) };
					return done.map(|value| value.expect("ready"));
				}
			}
			if node
				.state
				.compare_exchange(POLLING, PARKED, Ordering::AcqRel, Ordering::Acquire)
				.is_ok()
			{
				// SAFETY: set when the module loaded.
				return Ok(unsafe { Obj::borrowed(PENDING.get().expect("loaded").ptr()) });
			}
			// Woken while it polled: the wake is for this poll's registration, so poll again.
		}
	})
}

/// `claim()`: take the future for one `await`, returning the token its wakes queue.
unsafe extern "C" fn future_claim(obj: *mut PyObject, _: *mut PyObject) -> *mut PyObject {
	// SAFETY: Python calls it on a `Future`, holding the GIL.
	let future = unsafe { &mut *obj.cast::<Future>() };
	wrap(|| match std::mem::replace(&mut future.claimed, true) {
		// SAFETY: a static exception type.
		true => raise(unsafe { PyExc_RuntimeError }, "a Rust future is awaited already"),
		false => int_out(future.token),
	})
}

unsafe extern "C" fn future_close_py(obj: *mut PyObject, _: *mut PyObject) -> *mut PyObject {
	// SAFETY: Python calls it on a `Future`, holding the GIL.
	unsafe { future_close(obj.cast()) };
	none().map_or(null_mut(), Obj::into_ptr)
}

unsafe extern "C" fn future_dealloc(obj: *mut PyObject) {
	// SAFETY: Python deallocates a `Future` once.
	unsafe {
		future_close(obj.cast());
		std::ptr::drop_in_place(&raw mut (*obj.cast::<Future>()).held);
		dealloc(obj);
	}
}

/// Whether `obj` is exactly the runtime's type in `slot`.
fn is_type(obj: *mut PyObject, slot: &std::sync::OnceLock<Static>) -> bool {
	// SAFETY: `obj` is live.
	slot.get().is_some_and(|t| unsafe { Py_TYPE(obj) }.cast() == t.ptr())
}

static OBJECT: std::sync::OnceLock<Static> = std::sync::OnceLock::new();
static FUTURE: std::sync::OnceLock<Static> = std::sync::OnceLock::new();
static QUEUE: std::sync::OnceLock<Static> = std::sync::OnceLock::new();
static NODE: std::sync::OnceLock<Static> = std::sync::OnceLock::new();

/// A method table entry.
pub const fn method(name: &'static CStr, f: PyCFunctionFast) -> PyMethodDef {
	PyMethodDef {
		ml_name: name.as_ptr(),
		ml_meth: PyMethodDefPointer { PyCFunctionFast: f },
		ml_flags: METH_FASTCALL,
		ml_doc: null(),
	}
}

const fn noargs(name: &'static CStr, f: PyCFunction) -> PyMethodDef {
	PyMethodDef {
		ml_name: name.as_ptr(),
		ml_meth: PyMethodDefPointer { PyCFunction: f },
		ml_flags: METH_NOARGS,
		ml_doc: null(),
	}
}

pub const END: PyMethodDef = PyMethodDef::zeroed();

/// A table of methods Python reads for the interpreter's life.
pub struct Methods<const N: usize>(pub UnsafeCell<[PyMethodDef; N]>);

// SAFETY: Python reads it, holding the GIL.
unsafe impl<const N: usize> Sync for Methods<N> {}

impl<const N: usize> Methods<N> {
	pub fn ptr(&self) -> *mut PyMethodDef {
		self.0.get().cast()
	}
}

static OBJECT_METHODS: Methods<4> = Methods(UnsafeCell::new([
	noargs(c"close", handle_close),
	noargs(c"__enter__", handle_enter),
	method(c"__exit__", handle_exit),
	END,
]));

static FUTURE_METHODS: Methods<4> = Methods(UnsafeCell::new([
	method(c"poll", future_poll),
	noargs(c"claim", future_claim),
	noargs(c"close", future_close_py),
	END,
]));

static QUEUE_METHODS: Methods<3> = Methods(UnsafeCell::new([
	noargs(c"fileno", queue_fileno),
	noargs(c"take", queue_take),
	END,
]));

static NODE_METHODS: Methods<2> = Methods(UnsafeCell::new([noargs(c"token", node_token), END]));

static REGISTER: Methods<2> = Methods(UnsafeCell::new([method(c"_register", register), END]));

/// Make a type from `slots` and add it to `module` by its name, noting it in `cell`.
///
/// # Safety
/// The GIL is held, and `module` is the module being made.
unsafe fn add_type(
	module: *mut PyObject,
	cell: &std::sync::OnceLock<Static>,
	name: &CStr,
	size: usize,
	flags: u64,
	slots: &mut [PyType_Slot],
) -> R<()> {
	let qualified = format!("{}.{}\0", module_name(module), name.to_str().expect("ASCII"));
	// Python keeps the name for the type's life.
	let qualified: &'static str = Box::leak(qualified.into_boxed_str());
	let mut spec = PyType_Spec {
		name: qualified.as_ptr().cast(),
		basicsize: size as c_int,
		itemsize: 0,
		flags: (Py_TPFLAGS_DEFAULT as u64 | flags) as _,
		slots: slots.as_mut_ptr(),
	};
	// SAFETY: the caller's contract, and `spec` a valid spec.
	unsafe {
		let class = Obj::new(PyType_FromSpec(&mut spec))?;
		if PyModule_AddObjectRef(module, name.as_ptr(), class.ptr()) != 0 {
			return Err(Raised);
		}
		let _ = cell.set(Static(class));
	}
	Ok(())
}

fn module_name(module: *mut PyObject) -> String {
	// SAFETY: the GIL is held, and `module` a module.
	unsafe {
		let name = PyModule_GetName(module);
		CStr::from_ptr(name).to_string_lossy().into_owned()
	}
}

/// Make the extension module `def` names, with the runtime's types and functions beside
/// the glue's.
///
/// # Safety
/// Python calls the module's `PyInit_` with the GIL held, once per interpreter.
pub unsafe fn module(def: *mut PyModuleDef) -> *mut PyObject {
	// SAFETY: the caller's contract.
	unsafe {
		let Ok(module) = Obj::new(PyModule_Create(def)) else {
			return null_mut();
		};
		match runtime(module.ptr()) {
			Ok(()) => module.into_ptr(),
			Err(Raised) => null_mut(),
		}
	}
}

unsafe fn runtime(module: *mut PyObject) -> R<()> {
	// SAFETY: the GIL is held, and `module` the module being made.
	unsafe {
		if PyModule_AddFunctions(module, REGISTER.ptr()) != 0 {
			return Err(Raised);
		}
		let base = (Py_TPFLAGS_BASETYPE | Py_TPFLAGS_DISALLOW_INSTANTIATION) as u64;
		add_type(module, &OBJECT, c"Object", size_of::<Handle>(), base, &mut [
			slot(Py_tp_dealloc, handle_dealloc as *mut c_void),
			slot(Py_tp_methods, OBJECT_METHODS.ptr().cast()),
			slot(
				Py_tp_doc,
				c"An object owning a Rust value's handle, freed once the last reference to it goes, or sooner by close() or a with block.".as_ptr().cast_mut().cast(),
			),
			slot(0, null_mut()),
		])?;
		add_type(module, &FUTURE, c"Future", size_of::<Future>(), base, &mut [
			slot(Py_tp_dealloc, future_dealloc as *mut c_void),
			slot(Py_tp_methods, FUTURE_METHODS.ptr().cast()),
			slot(0, null_mut()),
		])?;
		add_type(module, &QUEUE, c"Queue", size_of::<QueueObj>(), 0, &mut [
			slot(Py_tp_new, queue_new as *mut c_void),
			slot(Py_tp_dealloc, queue_dealloc as *mut c_void),
			slot(Py_tp_methods, QUEUE_METHODS.ptr().cast()),
			slot(0, null_mut()),
		])?;
		add_type(module, &NODE, c"Node", size_of::<NodeObj>(), 0, &mut [
			slot(Py_tp_new, node_new as *mut c_void),
			slot(Py_tp_dealloc, node_obj_dealloc as *mut c_void),
			slot(Py_tp_methods, NODE_METHODS.ptr().cast()),
			slot(0, null_mut()),
		])?;
		let bytes_slots = &mut [
			slot(Py_bf_getbuffer, bytes_getbuffer as *mut c_void),
			slot(Py_tp_dealloc, bytes_dealloc as *mut c_void),
			slot(0, null_mut()),
		];
		add_type(module, &BYTES, c"Bytes", size_of::<Bytes>(), Py_TPFLAGS_DISALLOW_INSTANTIATION as u64, bytes_slots)?;
		// A plain `object()`, which nothing else is.
		let pending = Obj::new(PyObject_CallObject((&raw mut PyBaseObject_Type).cast(), null_mut()))?;
		if PyModule_AddObjectRef(module, c"PENDING".as_ptr(), pending.ptr()) != 0 {
			return Err(Raised);
		}
		let _ = PENDING.set(Static(pending));
	}
	Ok(())
}

const fn slot(slot: c_int, pfunc: *mut c_void) -> PyType_Slot {
	PyType_Slot { slot, pfunc }
}
