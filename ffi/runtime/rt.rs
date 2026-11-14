//! The runtime every export shares: buffers, handles, lent functions, wakers, and futures
//! across the C ABI. rust2-ffi copies it beside the generated code.
//!
//! A broken contract across the ABI (a null or consumed handle, a handle borrowed in a
//! way Rust's rules forbid, invalid UTF-8, a future polled after it finished) aborts with
//! a message naming the C function: there is no Rust caller to return an error to.

use std::cell::UnsafeCell;
use std::ffi::c_void;
use std::mem::MaybeUninit;
use std::ops::{Deref, DerefMut};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::task::{Context, Wake};

/// Abort with a message naming the C function whose contract was broken.
#[cold]
pub fn fail(what: &str, msg: &str) -> ! {
	eprintln!("{what}: {msg}");
	std::process::abort()
}

/// `s` for as long as the program runs, for a parameter that must be `'static`: each
/// distinct string is leaked once and lent to every later call with equal content, so the
/// strings held grow only with the distinct ones passed.
pub fn intern(s: &str) -> &'static str {
	static INTERNED: std::sync::Mutex<std::collections::BTreeSet<&'static str>> =
		std::sync::Mutex::new(std::collections::BTreeSet::new());
	let mut interned = INTERNED.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
	if let Some(&s) = interned.get(s) {
		return s;
	}
	let s: &'static str = Box::leak(Box::from(s));
	interned.insert(s);
	s
}

/// `b` for as long as the program runs, as [`intern`] keeps a string: keyed by its
/// contents, never by the caller's pointer, which the caller may free or reuse.
pub fn intern_bytes(b: &[u8]) -> &'static [u8] {
	static INTERNED: std::sync::Mutex<std::collections::BTreeSet<&'static [u8]>> =
		std::sync::Mutex::new(std::collections::BTreeSet::new());
	let mut interned = INTERNED.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
	if let Some(&b) = interned.get(b) {
		return b;
	}
	let b: &'static [u8] = Box::leak(Box::from(b));
	interned.insert(b);
	b
}

/// An `Arc` of another crate's type, held erased behind a handle with none of its API, so
/// the export names neither the type nor its crate. Rust takes it back as the `Arc` it
/// gave, and a handle of another such type aborts. A clone clones the `Arc` itself, so
/// Rust counts each handle as an owner.
pub struct Opaque(Box<dyn Held>);

/// An `Arc<T>` of any `T`, a trait object's too, erased.
trait Held: Send + Sync {
	fn any(&self) -> &dyn std::any::Any;
	fn clone_box(&self) -> Box<dyn Held>;
}

impl<T: ?Sized + Send + Sync + 'static> Held for Arc<T> {
	fn any(&self) -> &dyn std::any::Any {
		self
	}

	fn clone_box(&self) -> Box<dyn Held> {
		Box::new(Arc::clone(self))
	}
}

impl Clone for Opaque {
	fn clone(&self) -> Self {
		Self(self.0.clone_box())
	}
}

impl Opaque {
	pub fn new<T: ?Sized + Send + Sync + 'static>(value: Arc<T>) -> Self {
		Self(Box::new(value))
	}

	/// The `Arc` it holds, lent.
	pub fn get<T: ?Sized + Send + Sync + 'static>(&self, what: &str) -> &Arc<T> {
		(self.0.any().downcast_ref())
			.unwrap_or_else(|| fail(what, "an opaque handle holds another type"))
	}

	/// A clone of the `Arc` it holds.
	pub fn arc<T: ?Sized + Send + Sync + 'static>(&self, what: &str) -> Arc<T> {
		Arc::clone(self.get(what))
	}
}

/// An argument a future borrows, kept in place on the heap until the future is dropped.
pub struct Pinned<T>(std::ptr::NonNull<T>);

// It owns its `T`, as a `Box` does.
unsafe impl<T: Send> Send for Pinned<T> {}
unsafe impl<T: Sync> Sync for Pinned<T> {}

impl<T> Pinned<T> {
	pub fn new(value: T) -> Self {
		Self(std::ptr::NonNull::from(Box::leak(Box::new(value))))
	}

	/// The value, lent past this borrow of its holder.
	///
	/// # Safety
	///
	/// Called once, and the borrow ends before the holder drops, as [`Holding`] ensures.
	pub unsafe fn get(&self) -> &'static mut T {
		unsafe { &mut *self.0.as_ptr() }
	}
}

impl<T> Drop for Pinned<T> {
	fn drop(&mut self) {
		drop(unsafe { Box::from_raw(self.0.as_ptr()) });
	}
}

/// A future and the arguments it borrows, which drop after it: fields drop in order.
pub struct Holding<F, H> {
	future: F,
	_held: H,
}

impl<F, H> Holding<F, H> {
	pub fn new(future: F, held: H) -> Self {
		Self { future, _held: held }
	}
}

impl<F: std::future::Future, H> std::future::Future for Holding<F, H> {
	type Output = F::Output;

	fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<F::Output> {
		// The future is pinned with its holder and never moves out; the held values are
		// never pinned.
		unsafe { self.map_unchecked_mut(|holding| &mut holding.future) }.poll(cx)
	}
}

/// Another crate's value, held erased by its handle, which the glue never names.
pub struct Owned(Box<dyn std::any::Any + Send + Sync>);

impl Owned {
	pub fn new<T: Send + Sync + 'static>(value: T) -> Self {
		Self(Box::new(value))
	}

	/// The value it holds, lent.
	pub fn get<T: 'static>(&self, what: &str) -> &T {
		(self.0.downcast_ref()).unwrap_or_else(|| fail(what, "an opaque handle holds another type"))
	}

	/// The value it holds, lent exclusively.
	pub fn get_mut<T: 'static>(&mut self, what: &str) -> &mut T {
		(self.0.downcast_mut()).unwrap_or_else(|| fail(what, "an opaque handle holds another type"))
	}

	/// The value it holds, taken back.
	pub fn take<T: 'static>(self, what: &str) -> T {
		*(self.0.downcast()).unwrap_or_else(|_| fail(what, "an opaque handle holds another type"))
	}
}

/// Bytes across the ABI. Returned, an owned buffer the caller frees once. Passed in by
/// value, one with an owner is Rust's own, which it takes without a copy, and one without
/// is a view it copies or borrows for the call; behind a pointer, as an array's items are,
/// it is only read. A null `ptr` is `None`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Buf {
	pub ptr: *const u8,
	pub len: usize,
	pub owner: *mut c_void,
}

impl Buf {
	pub const NONE: Self = Self {
		ptr: std::ptr::null(),
		len: 0,
		owner: std::ptr::null_mut(),
	};

	pub fn is_none(&self) -> bool {
		self.ptr.is_null()
	}

	/// The bytes a caller passed in, which it keeps, borrowed no longer than the buffer
	/// is, so nothing kept past the call can hold them.
	///
	/// # Safety
	/// `ptr` points at `len` readable bytes while `self` lives.
	pub unsafe fn bytes(&self, what: &str) -> &[u8] {
		match (self.ptr.is_null(), self.len) {
			(_, 0) => &[],
			(true, _) => fail(what, "a buffer has a length but no pointer"),
			// SAFETY: the caller's contract.
			(false, len) => unsafe { std::slice::from_raw_parts(self.ptr, len) },
		}
	}

	/// The bytes a caller passed in: taken without a copy where `take` and it has an
	/// owner, else copied, leaving the caller's buffer as it was.
	///
	/// # Safety
	/// As [`Self::bytes`], and an owner, where `take`, came from [`Self::owned`] and is
	/// freed by nothing else.
	pub unsafe fn bytes_in(self, what: &str, take: bool) -> bytes::Bytes {
		match take && !self.owner.is_null() {
			// SAFETY: the caller's contract.
			true => unsafe { self.take(what) },
			// SAFETY: the caller's contract.
			false => bytes::Bytes::copy_from_slice(unsafe { self.bytes(what) }),
		}
	}

	/// As [`Self::bytes_in`], as a `Vec`, which reuses an owner's allocation it alone holds.
	///
	/// # Safety
	/// As [`Self::bytes_in`].
	pub unsafe fn vec_in(self, what: &str, take: bool) -> Vec<u8> {
		match take && !self.owner.is_null() {
			// SAFETY: the caller's contract.
			true => Vec::from(unsafe { self.take(what) }),
			// SAFETY: the caller's contract.
			false => unsafe { self.bytes(what) }.to_vec(),
		}
	}

	/// As [`Self::vec_in`], checked to be UTF-8.
	///
	/// # Safety
	/// As [`Self::bytes_in`].
	pub unsafe fn string_in(self, what: &str, take: bool) -> String {
		// SAFETY: the caller's contract.
		match String::from_utf8(unsafe { self.vec_in(what, take) }) {
			Ok(s) => s,
			Err(_) => fail(what, "a string is not UTF-8"),
		}
	}

	/// As [`Self::vec_in`], as a path: its bytes as the OS holds them on Unix, else UTF-8.
	///
	/// # Safety
	/// As [`Self::bytes_in`].
	pub unsafe fn path_in(self, what: &str, take: bool) -> std::path::PathBuf {
		#[cfg(unix)]
		{
			use std::os::unix::ffi::OsStringExt;
			// SAFETY: the caller's contract.
			std::path::PathBuf::from(std::ffi::OsString::from_vec(unsafe { self.vec_in(what, take) }))
		}
		#[cfg(not(unix))]
		{
			// SAFETY: the caller's contract.
			std::path::PathBuf::from(unsafe { self.string_in(what, take) })
		}
	}

	/// The bytes a caller passed in for the call alone: taken where it has an owner, which
	/// they free after, else borrowed.
	///
	/// # Safety
	/// As [`Self::bytes_in`] taking.
	pub unsafe fn hold<'a>(self, what: &str) -> Hold<'a> {
		match self.owner.is_null() {
			// SAFETY: the caller's contract; a view lives as long as the call.
			true => Hold::Lent(unsafe { std::slice::from_raw_parts(self.bytes(what).as_ptr(), self.len) }),
			// SAFETY: the caller's contract.
			false => Hold::Owned(unsafe { self.take(what) }),
		}
	}

	/// The bytes an owned buffer holds, taken from its owner: `ptr` and `len` may be any
	/// range of them, as a caller slices a buffer Rust returned.
	///
	/// # Safety
	/// `owner` came from [`Self::owned`], and is freed by nothing else.
	unsafe fn take(self, what: &str) -> bytes::Bytes {
		// SAFETY: the caller's contract.
		let bytes = *unsafe { Box::from_raw(self.owner.cast::<bytes::Bytes>()) };
		let start = (self.ptr as usize).wrapping_sub(bytes.as_ptr() as usize);
		match self.len {
			0 => bytes::Bytes::new(),
			// Whole, as is: a slice of bytes made from a `Vec` would allocate to share them.
			len if start == 0 && len == bytes.len() => bytes,
			_ if start <= bytes.len() && self.len <= bytes.len() - start => bytes.slice(start..start + self.len),
			_ => fail(what, "a buffer's bytes are not its owner's"),
		}
	}

	/// A second owner of an owned buffer's bytes, sharing them: what a caller passes Rust
	/// while it keeps the buffer.
	///
	/// # Safety
	/// `owner` came from [`Self::owned`], and is not yet freed.
	pub unsafe fn share(self, what: &str) -> Self {
		if self.owner.is_null() {
			fail(what, "a buffer has no owner to share");
		}
		// SAFETY: the caller's contract.
		let bytes = unsafe { &*self.owner.cast::<bytes::Bytes>() };
		Self {
			owner: Box::into_raw(Box::new(bytes.clone())).cast(),
			..self
		}
	}

	/// The string a caller passed in, which it keeps.
	///
	/// # Safety
	/// As [`Self::bytes`].
	pub unsafe fn str(&self, what: &str) -> &str {
		// SAFETY: the caller's contract.
		match std::str::from_utf8(unsafe { self.bytes(what) }) {
			Ok(s) => s,
			Err(_) => fail(what, "a string is not UTF-8"),
		}
	}

	/// A view of bytes Rust keeps, for a callback to read during its call.
	pub fn view(bytes: &[u8]) -> Self {
		Self {
			ptr: bytes.as_ptr(),
			len: bytes.len(),
			owner: std::ptr::null_mut(),
		}
	}

	/// Bytes handed to the caller, which frees them.
	pub fn owned(bytes: bytes::Bytes) -> Self {
		let ptr = bytes.as_ptr();
		let len = bytes.len();
		let owner = Box::into_raw(Box::new(bytes)).cast();
		Self { ptr, len, owner }
	}

	pub fn string(string: String) -> Self {
		Self::owned(string.into())
	}

	pub fn vec(vec: Vec<u8>) -> Self {
		Self::owned(vec.into())
	}

	/// The path a caller passed in, which it keeps: its bytes as the OS holds them on
	/// Unix, which may be any, else UTF-8.
	///
	/// # Safety
	/// As [`Self::bytes`].
	pub unsafe fn path(&self, what: &str) -> &std::path::Path {
		#[cfg(unix)]
		{
			use std::os::unix::ffi::OsStrExt;
			// SAFETY: the caller's contract.
			std::path::Path::new(std::ffi::OsStr::from_bytes(unsafe { self.bytes(what) }))
		}
		#[cfg(not(unix))]
		{
			// SAFETY: the caller's contract.
			std::path::Path::new(unsafe { self.str(what) })
		}
	}

	/// A path handed to the caller: its bytes as the OS holds them on Unix, else as
	/// Rust encodes them, which is UTF-8 for any path that has a UTF-8 form.
	pub fn path_buf(path: std::path::PathBuf) -> Self {
		Self::vec(path.into_os_string().into_encoded_bytes())
	}

	/// The buffer `owner` holds, as [`Self::owned`] returned it.
	///
	/// # Safety
	/// `owner` came from [`Self::owned`], and is not yet freed.
	pub unsafe fn of_owner(owner: *mut c_void) -> Self {
		// SAFETY: the caller's contract.
		let bytes = unsafe { &*owner.cast::<bytes::Bytes>() };
		Self {
			ptr: bytes.as_ptr(),
			len: bytes.len(),
			owner,
		}
	}

	/// The string in a buffer C returned Rust, as `<prefix>_buf_copy` made it, which Rust
	/// now owns.
	///
	/// # Safety
	/// `owner` is null or came from [`Self::owned`], and is not yet freed.
	pub unsafe fn take_string(self, what: &str) -> String {
		if self.owner.is_null() {
			fail(what, "a returned buffer is not one buf_copy made");
		}
		// SAFETY: the caller's contract.
		unsafe { self.string_in(what, true) }
	}

	/// Free a buffer Rust returned; a view or `None` frees nothing.
	///
	/// # Safety
	/// `owner` is null or came from [`Self::owned`], and is freed once.
	pub unsafe fn free(self) {
		if !self.owner.is_null() {
			// SAFETY: the caller's contract.
			drop(unsafe { Box::from_raw(self.owner.cast::<bytes::Bytes>()) });
		}
	}
}

/// Bytes passed in for a call: a view borrowed for it, or a buffer Rust took, freed after.
pub enum Hold<'a> {
	Lent(&'a [u8]),
	Owned(bytes::Bytes),
}

impl Hold<'_> {
	pub fn bytes(&self) -> &[u8] {
		match self {
			Self::Lent(bytes) => bytes,
			Self::Owned(bytes) => bytes,
		}
	}

	pub fn str(&self, what: &str) -> &str {
		match std::str::from_utf8(self.bytes()) {
			Ok(s) => s,
			Err(_) => fail(what, "a string is not UTF-8"),
		}
	}

	/// The path these hold: its bytes as the OS holds them on Unix, else UTF-8.
	pub fn path(&self, what: &str) -> &std::path::Path {
		#[cfg(unix)]
		{
			use std::os::unix::ffi::OsStrExt;
			let _ = what;
			std::path::Path::new(std::ffi::OsStr::from_bytes(self.bytes()))
		}
		#[cfg(not(unix))]
		{
			std::path::Path::new(self.str(what))
		}
	}
}

/// `u128`, as its two halves.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct U128 {
	pub lo: u64,
	pub hi: u64,
}

impl U128 {
	pub fn lift(self) -> u128 {
		u128::from(self.hi) << 64 | u128::from(self.lo)
	}

	pub fn lower(v: u128) -> Self {
		Self {
			lo: v as u64,
			hi: (v >> 64) as u64,
		}
	}
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Duration {
	pub secs: u64,
	pub nanos: u32,
}

impl Duration {
	pub fn lift(self, what: &str) -> std::time::Duration {
		if self.nanos >= 1_000_000_000 {
			fail(what, "a duration has a billion nanoseconds or more");
		}
		std::time::Duration::new(self.secs, self.nanos)
	}

	pub fn lower(duration: std::time::Duration) -> Self {
		Self {
			secs: duration.as_secs(),
			nanos: duration.subsec_nanos(),
		}
	}
}

/// `std::net::SocketAddr`: an IPv4 address in the first 4 bytes of `ip`, with the rest of
/// `ip`, `flowinfo`, and `scope_id` zero, or an IPv6 address. `ip` is in network order and
/// `port` in the host's.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SocketAddr {
	pub ip: [u8; 16],
	pub port: u16,
	pub v6: bool,
	pub flowinfo: u32,
	pub scope_id: u32,
}

impl SocketAddr {
	pub fn lift(self, what: &str) -> std::net::SocketAddr {
		if self.v6 {
			let ip = std::net::Ipv6Addr::from(self.ip);
			return std::net::SocketAddrV6::new(ip, self.port, self.flowinfo, self.scope_id).into();
		}
		let [a, b, c, d, rest @ ..] = self.ip;
		if rest != [0; 12] || self.flowinfo != 0 || self.scope_id != 0 {
			fail(what, "an IPv4 socket address has IPv6 bytes or fields");
		}
		std::net::SocketAddrV4::new(std::net::Ipv4Addr::new(a, b, c, d), self.port).into()
	}

	/// An IPv4 address, for `SocketAddrV4`: an IPv6 one aborts.
	pub fn lift_v4(self, what: &str) -> std::net::SocketAddrV4 {
		match self.lift(what) {
			std::net::SocketAddr::V4(addr) => addr,
			std::net::SocketAddr::V6(_) => fail(what, "an IPv6 socket address where IPv4 is taken"),
		}
	}

	/// An IPv6 address, for `SocketAddrV6`: an IPv4 one aborts.
	pub fn lift_v6(self, what: &str) -> std::net::SocketAddrV6 {
		match self.lift(what) {
			std::net::SocketAddr::V6(addr) => addr,
			std::net::SocketAddr::V4(_) => fail(what, "an IPv4 socket address where IPv6 is taken"),
		}
	}

	pub fn lower(addr: std::net::SocketAddr) -> Self {
		match addr {
			std::net::SocketAddr::V4(v4) => {
				let mut ip = [0; 16];
				ip[..4].copy_from_slice(&v4.ip().octets());
				Self {
					ip,
					port: v4.port(),
					v6: false,
					flowinfo: 0,
					scope_id: 0,
				}
			}
			std::net::SocketAddr::V6(v6) => Self {
				ip: v6.ip().octets(),
				port: v6.port(),
				v6: true,
				flowinfo: v6.flowinfo(),
				scope_id: v6.scope_id(),
			},
		}
	}
}

/// `Vec<T>` across the ABI, as an array of its elements' C values. Passed in, a view Rust
/// copies each element out of and never frees. Returned, an array the caller frees once,
/// with what its first `len` elements hold. A null `ptr` is `None`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct List<T> {
	pub ptr: *const T,
	pub len: usize,
	pub owner: *mut c_void,
}

impl<T: Copy> List<T> {
	pub const NONE: Self = Self {
		ptr: std::ptr::null(),
		len: 0,
		owner: std::ptr::null_mut(),
	};

	pub fn is_none(&self) -> bool {
		self.ptr.is_null()
	}

	/// The elements a caller passed in, which it keeps.
	///
	/// # Safety
	/// `ptr` points at `len` readable elements while `self` lives.
	pub unsafe fn items(&self, what: &str) -> &[T] {
		match (self.ptr.is_null(), self.len) {
			(_, 0) => &[],
			(true, _) => fail(what, "an array has a length but no pointer"),
			// SAFETY: the caller's contract.
			(false, len) => unsafe { std::slice::from_raw_parts(self.ptr, len) },
		}
	}

	/// Elements handed to the caller, which frees them.
	pub fn owned(items: Vec<T>) -> Self {
		let items = Box::new(items);
		Self {
			ptr: items.as_ptr(),
			len: items.len(),
			owner: Box::into_raw(items).cast(),
		}
	}

	/// Free an array Rust returned, after `each` frees what each of its first `len`
	/// elements holds: a binding that took the elements frees the array with `len` 0.
	///
	/// # Safety
	/// `owner` is null or came from [`Self::owned`], and is freed once.
	pub unsafe fn free(self, what: &str, each: impl Fn(T)) {
		if self.owner.is_null() {
			return;
		}
		// SAFETY: the caller's contract.
		let items = unsafe { Box::from_raw(self.owner.cast::<Vec<T>>()) };
		if !std::ptr::eq(self.ptr, items.as_ptr()) || self.len > items.len() {
			fail(what, "an array is not as Rust returned it");
		}
		items[..self.len].iter().copied().for_each(each);
	}
}

/// `std::ops::Range`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Range<T> {
	pub start: T,
	pub end: T,
}

impl<T> Range<T> {
	pub fn lift(self) -> std::ops::Range<T> {
		self.start..self.end
	}

	pub fn lower(range: std::ops::Range<T>) -> Self {
		Self {
			start: range.start,
			end: range.end,
		}
	}
}

/// `std::ops::Bound`: its kind, and the value of an included or excluded bound.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Bound<T> {
	pub kind: i32,
	pub value: T,
}

const UNBOUNDED: i32 = 0;
const INCLUDED: i32 = 1;
const EXCLUDED: i32 = 2;

impl<T: Default> Bound<T> {
	pub fn lift(self, what: &str) -> std::ops::Bound<T> {
		match self.kind {
			UNBOUNDED => std::ops::Bound::Unbounded,
			INCLUDED => std::ops::Bound::Included(self.value),
			EXCLUDED => std::ops::Bound::Excluded(self.value),
			_ => fail(what, "a bound's kind is invalid"),
		}
	}

	pub fn lower(bound: std::ops::Bound<T>) -> Self {
		let (kind, value) = match bound {
			std::ops::Bound::Unbounded => (UNBOUNDED, T::default()),
			std::ops::Bound::Included(value) => (INCLUDED, value),
			std::ops::Bound::Excluded(value) => (EXCLUDED, value),
		};
		Self { kind, value }
	}
}

/// `(Bound<T>, Bound<T>)`, the form a `RangeBounds<T>` parameter takes.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Bounds<T> {
	pub start: Bound<T>,
	pub end: Bound<T>,
}

impl<T: Default> Bounds<T> {
	pub fn lift(self, what: &str) -> (std::ops::Bound<T>, std::ops::Bound<T>) {
		(self.start.lift(what), self.end.lift(what))
	}

	pub fn lower((start, end): (std::ops::Bound<T>, std::ops::Bound<T>)) -> Self {
		Self {
			start: Bound::lower(start),
			end: Bound::lower(end),
		}
	}
}

pub fn char(code: u32, what: &str) -> char {
	char::from_u32(code).unwrap_or_else(|| fail(what, "a char is not a Unicode scalar value"))
}

const EXCLUSIVE: usize = usize::MAX;
const TAKEN: usize = usize::MAX - 1;

/// A value behind a handle: shared through an `Arc`, and borrowed as Rust borrows it, with
/// the borrow checked at run time. `state` counts shared borrows, or is [`EXCLUSIVE`]
/// while one exclusive borrow lasts, or [`TAKEN`] once the value moved out. A value that is
/// not `Sync` is shared on one thread at a time instead: `state` is that thread's id, and
/// `depth` counts its borrows, which only that thread touches.
pub struct Object<T> {
	state: AtomicUsize,
	depth: UnsafeCell<usize>,
	/// The value a lent handle points at, where Rust lent it, or null if it owns `value`.
	lent: *mut T,
	value: UnsafeCell<MaybeUninit<T>>,
	/// Bumped each time the value is borrowed exclusively or moved out, or the handle is
	/// freed, so no handle lent from the value outlives any of them.
	epoch: AtomicUsize,
	/// For a handle a call returned lent from another's value, that handle.
	lender: Option<Lender>,
}

/// The handle a returned borrow was lent from, and its epoch then: the borrow lives while
/// the epoch stands, and each use holds the handle shared, so none can start meanwhile. A
/// value that is not `Sync` is held `local`ly, on the using thread alone.
pub struct Lender {
	handle: Arc<dyn Lends>,
	epoch: usize,
	local: bool,
}

/// A handle another is lent from.
trait Lends: Send + Sync {
	/// Borrow the value shared for a use of a handle lent from it, if still at `epoch`.
	fn hold(&self, epoch: usize, local: bool, what: &str);
	/// End what [`Self::hold`] took.
	fn unhold(&self, local: bool);
}

impl<T: Send> Lends for Object<T> {
	fn hold(&self, epoch: usize, local: bool, what: &str) {
		match local {
			true => self.acquire_local(what),
			false => self.acquire(what),
		}
		if self.epoch.load(Ordering::Acquire) != epoch {
			self.unhold(local);
			fail(
				what,
				"a handle lent from another outlived it: the other was borrowed exclusively, moved out, or freed",
			);
		}
	}

	fn unhold(&self, local: bool) {
		match local {
			true => self.release_local(),
			false => Object::release(self),
		}
	}
}

/// This thread's id for [`Object::borrow_local`]: the address of a thread local, which no
/// other live thread shares and which is neither 0 nor a sentinel. A thread Dart's runtime
/// hands a call to, while the thread it acts for waits, takes that thread's.
pub fn thread() -> usize {
	#[cfg(feature = "dart")]
	if let Some(id) = crate::dart::acting() {
		return id;
	}
	thread_local!(static ID: u8 = const { 0 });
	ID.with(|id| std::ptr::from_ref(id) as usize)
}

// SAFETY: the value moves between threads only by `take` or an exclusive borrow, and is
// shared across threads only by `borrow` and `borrow_arc`, which require `Sync`.
unsafe impl<T: Send> Send for Object<T> {}
// SAFETY: as above.
unsafe impl<T: Send> Sync for Object<T> {}

impl<T> Object<T> {
	/// A new handle owning `value`, which may outlive any borrow, so `value` holds none.
	pub fn new(value: T) -> *const c_void
	where
		T: 'static,
	{
		Arc::into_raw(Arc::new(Self {
			state: AtomicUsize::new(0),
			depth: UnsafeCell::new(0),
			lent: std::ptr::null_mut(),
			value: UnsafeCell::new(MaybeUninit::new(value)),
			epoch: AtomicUsize::new(0),
			lender: None,
		}))
		.cast()
	}

	/// A handle to `value`, which a call returned lent from `lender`'s value: shared, as a
	/// returned `&T` is, and borrowed as its own type is, until that value is borrowed
	/// exclusively, moved out, or freed, after which a use aborts. It is never borrowed
	/// exclusively nor moved out itself.
	pub fn lent(value: &T, lender: Lender) -> *const c_void {
		Arc::into_raw(Arc::new(Self {
			state: AtomicUsize::new(0),
			depth: UnsafeCell::new(0),
			lent: std::ptr::from_ref(value).cast_mut(),
			value: UnsafeCell::new(MaybeUninit::uninit()),
			epoch: AtomicUsize::new(0),
			lender: Some(lender),
		}))
		.cast()
	}

	/// The handle a call borrowing it shared returns a borrow lent from, as a [`Lender`].
	///
	/// # Safety
	/// `handle` is a live handle of this type.
	pub unsafe fn lender(handle: *const c_void, what: &str) -> Lender
	where
		T: Send + Sync + 'static,
	{
		// SAFETY: the caller's contract.
		unsafe { Self::lender_of(handle, false, what) }
	}

	/// [`Self::lender`] of a value that is not `Sync`, held on one thread at a time.
	///
	/// # Safety
	/// As [`Self::lender`].
	pub unsafe fn lender_local(handle: *const c_void, what: &str) -> Lender
	where
		T: Send + 'static,
	{
		// SAFETY: the caller's contract.
		unsafe { Self::lender_of(handle, true, what) }
	}

	/// # Safety
	/// As [`Self::lender`].
	unsafe fn lender_of(handle: *const c_void, local: bool, what: &str) -> Lender
	where
		T: Send + 'static,
	{
		// SAFETY: the caller's contract.
		let object = unsafe { Self::arc(handle, what) };
		let epoch = object.epoch.load(Ordering::Acquire);
		Lender { handle: object, epoch, local }
	}

	/// The value: its own, or the one it was lent.
	fn value(&self) -> *mut T {
		match self.lent.is_null() {
			true => self.value.get().cast(),
			false => self.lent,
		}
	}

	/// # Safety
	/// `handle` is null or a live handle of this type.
	unsafe fn get<'a>(handle: *const c_void, what: &str) -> &'a Self {
		if handle.is_null() {
			fail(what, "a handle is null");
		}
		// SAFETY: the caller's contract.
		unsafe { &*handle.cast::<Self>() }
	}

	fn acquire(&self, what: &str) {
		let mut state = self.state.load(Ordering::Relaxed);
		loop {
			match state {
				TAKEN => fail(what, "a handle's value was moved out"),
				EXCLUSIVE => fail(what, "a handle is borrowed exclusively"),
				_ => {}
			}
			match self
				.state
				.compare_exchange_weak(state, state + 1, Ordering::Acquire, Ordering::Relaxed)
			{
				Ok(_) => break,
				Err(now) => state = now,
			}
		}
		self.hold_lender(what);
	}

	/// End a shared borrow [`Self::acquire`] took.
	fn release(&self) {
		self.release_lender();
		self.state.fetch_sub(1, Ordering::Release);
	}

	/// Borrow a value that is not `Sync` shared: any number of times on one thread, as
	/// Rust allows, and on no other thread meanwhile.
	fn acquire_local(&self, what: &str) {
		let me = thread();
		match self.state.compare_exchange(0, me, Ordering::Acquire, Ordering::Relaxed) {
			Ok(_) => {}
			Err(owner) if owner == me => {}
			Err(TAKEN) => fail(what, "a handle's value was moved out"),
			Err(EXCLUSIVE) => fail(what, "a handle is borrowed exclusively"),
			Err(_) => fail(what, "a handle is borrowed on another thread"),
		}
		// SAFETY: only the thread in `state` touches `depth`.
		unsafe { *self.depth.get() += 1 };
		self.hold_lender(what);
	}

	/// End a shared borrow [`Self::acquire_local`] took, on its thread.
	fn release_local(&self) {
		self.release_lender();
		// SAFETY: this thread is the one in `state`, so `depth` is its own.
		let depth = unsafe { &mut *self.depth.get() };
		*depth -= 1;
		if *depth == 0 {
			self.state.store(0, Ordering::Release);
		}
	}

	/// For a handle lent from another, hold that one for a use of this one.
	fn hold_lender(&self, what: &str) {
		if let Some(lender) = &self.lender {
			lender.handle.hold(lender.epoch, lender.local, what);
		}
	}

	fn release_lender(&self) {
		if let Some(lender) = &self.lender {
			lender.handle.unhold(lender.local);
		}
	}

	fn acquire_mut(&self, what: &str) {
		if self.lender.is_some() {
			fail(what, "a handle a call lent shared is borrowed exclusively");
		}
		if let Err(state) = self
			.state
			.compare_exchange(0, EXCLUSIVE, Ordering::Acquire, Ordering::Relaxed)
		{
			match state {
				TAKEN => fail(what, "a handle's value was moved out"),
				_ => fail(what, "a handle is already borrowed"),
			}
		}
		self.epoch.fetch_add(1, Ordering::Release);
	}

	/// Borrow the value shared for a call.
	///
	/// # Safety
	/// `handle` is a live handle of this type, kept alive through the borrow.
	pub unsafe fn borrow<'a>(handle: *const c_void, what: &str) -> Shared<'a, T>
	where
		T: Sync,
	{
		// SAFETY: the caller's contract.
		let object = unsafe { Self::get(handle, what) };
		object.acquire(what);
		Shared { object }
	}

	/// Borrow a value that is not `Sync` shared for a call: any number of times on one
	/// thread, as Rust allows, and on no other thread meanwhile.
	///
	/// # Safety
	/// As [`Self::borrow`].
	pub unsafe fn borrow_local<'a>(handle: *const c_void, what: &str) -> Local<'a, T> {
		// SAFETY: the caller's contract.
		let object = unsafe { Self::get(handle, what) };
		object.acquire_local(what);
		Local { object }
	}

	/// Borrow the value exclusively for a call.
	///
	/// # Safety
	/// As [`Self::borrow`].
	pub unsafe fn borrow_mut<'a>(handle: *const c_void, what: &str) -> Exclusive<'a, T> {
		// SAFETY: the caller's contract.
		let object = unsafe { Self::get(handle, what) };
		object.acquire_mut(what);
		Exclusive { object }
	}

	/// Borrow the value shared for as long as a future runs, keeping the handle alive.
	///
	/// # Safety
	/// `handle` is a live handle of this type.
	pub unsafe fn borrow_arc(handle: *const c_void, what: &str) -> SharedArc<T>
	where
		T: Sync,
	{
		// SAFETY: the caller's contract.
		let object = unsafe { Self::arc(handle, what) };
		object.acquire(what);
		SharedArc { object }
	}

	/// Borrow the value exclusively for as long as a future runs.
	///
	/// # Safety
	/// As [`Self::borrow_arc`].
	pub unsafe fn borrow_mut_arc(handle: *const c_void, what: &str) -> ExclusiveArc<T> {
		// SAFETY: the caller's contract.
		let object = unsafe { Self::arc(handle, what) };
		object.acquire_mut(what);
		ExclusiveArc { object }
	}

	/// # Safety
	/// `handle` is a live handle of this type.
	unsafe fn arc(handle: *const c_void, what: &str) -> Arc<Self> {
		// SAFETY: the caller's contract.
		unsafe { Self::get(handle, what) };
		let ptr = handle.cast::<Self>();
		// SAFETY: a live handle holds a count, so one more keeps it alive.
		unsafe {
			Arc::increment_strong_count(ptr);
			Arc::from_raw(ptr)
		}
	}

	/// Lend `value` to C as a handle while `f` runs, shared as `&T` lends it.
	pub fn lend<R>(value: &T, what: &str, f: impl FnOnce(*const c_void) -> R) -> R
	where
		T: Sync,
	{
		Self::lending(std::ptr::from_ref(value).cast_mut(), 1, 0, what, f)
	}

	/// Lend a value that is not `Sync` shared, as [`Self::lend`], on this thread alone.
	pub fn lend_local<R>(value: &T, what: &str, f: impl FnOnce(*const c_void) -> R) -> R {
		Self::lending(std::ptr::from_ref(value).cast_mut(), thread(), 1, what, f)
	}

	/// Lend `value` exclusively, as `&mut T` lends it.
	pub fn lend_mut<R>(value: &mut T, what: &str, f: impl FnOnce(*const c_void) -> R) -> R {
		Self::lending(std::ptr::from_mut(value), 0, 0, what, f)
	}

	/// Lend the value at `ptr` to C as a handle while `f` runs, holding the borrow `state`
	/// and `depth` describe, if any, so C borrows it no further than Rust lent it. C may
	/// neither move it out, nor free the handle, nor keep a borrow past `f`.
	fn lending<R>(ptr: *mut T, state: usize, depth: usize, what: &str, f: impl FnOnce(*const c_void) -> R) -> R {
		let object = Arc::new(Self {
			state: AtomicUsize::new(state),
			depth: UnsafeCell::new(depth),
			lent: ptr,
			value: UnsafeCell::new(MaybeUninit::uninit()),
			epoch: AtomicUsize::new(0),
			lender: None,
		});
		// A second count, which a free by C would take, so the handle outlives it.
		let spare = Arc::clone(&object);
		let result = f(Arc::as_ptr(&object).cast());
		match Arc::strong_count(&object) {
			2 if object.state.load(Ordering::Acquire) == state => {}
			1 => fail(what, "a lent handle was freed"),
			_ => fail(what, "a lent handle is still borrowed"),
		}
		drop(spare);
		result
	}

	/// Whether a call or a future borrows the value, so moving it out would abort. A value
	/// moved out is not, so the move refuses it, naming itself.
	///
	/// # Safety
	/// As [`Self::borrow`].
	pub unsafe fn borrowed(handle: *const c_void, what: &str) -> bool {
		// SAFETY: the caller's contract.
		let object = unsafe { Self::get(handle, what) };
		!matches!(object.state.load(Ordering::Acquire), 0 | TAKEN)
	}

	/// Move the value out, leaving the handle to be freed.
	///
	/// # Safety
	/// As [`Self::borrow`].
	pub unsafe fn take(handle: *const c_void, what: &str) -> T {
		// SAFETY: the caller's contract.
		let object = unsafe { Self::get(handle, what) };
		if !object.lent.is_null() {
			fail(what, "a lent handle's value cannot move out");
		}
		if let Err(state) = object
			.state
			.compare_exchange(0, TAKEN, Ordering::Acquire, Ordering::Relaxed)
		{
			match state {
				TAKEN => fail(what, "a handle's value was moved out"),
				_ => fail(what, "a handle is borrowed"),
			}
		}
		object.epoch.fetch_add(1, Ordering::Release);
		// SAFETY: TAKEN excludes every other access, and the value is never read again.
		unsafe { (*object.value.get()).assume_init_read() }
	}

	/// The value of a handle a closure C implements returned Rust, which copies it out and
	/// frees the handle.
	///
	/// # Safety
	/// `handle` is a live handle of this type, which C gives up.
	pub unsafe fn returned(handle: *const c_void, what: &str) -> T
	where
		T: Copy,
	{
		// SAFETY: the caller's contract.
		let value = *unsafe { Self::borrow_local(handle, what) };
		// SAFETY: as above; the borrow ended.
		unsafe { Self::free(handle) };
		value
	}

	/// Release the caller's handle.
	///
	/// # Safety
	/// `handle` is null or a live handle of this type, released once.
	pub unsafe fn free(handle: *const c_void) {
		if !handle.is_null() {
			// SAFETY: the caller's contract.
			let object = unsafe { Arc::from_raw(handle.cast::<Self>()) };
			// A future may still hold the value, but no handle lent from it outlives C's.
			object.epoch.fetch_add(1, Ordering::Release);
			drop(object);
		}
	}
}

impl<T> Drop for Object<T> {
	fn drop(&mut self) {
		if self.lent.is_null() && *self.state.get_mut() != TAKEN {
			// SAFETY: the last reference is gone, and the value is its own and never taken.
			unsafe { self.value.get_mut().assume_init_drop() };
		}
	}
}

pub struct Shared<'a, T> {
	object: &'a Object<T>,
}

impl<T> Deref for Shared<'_, T> {
	type Target = T;
	fn deref(&self) -> &T {
		// SAFETY: a shared borrow is held.
		unsafe { &*self.object.value() }
	}
}

impl<T> Drop for Shared<'_, T> {
	fn drop(&mut self) {
		self.object.release();
	}
}

pub struct Local<'a, T> {
	object: &'a Object<T>,
}

impl<T> Deref for Local<'_, T> {
	type Target = T;
	fn deref(&self) -> &T {
		// SAFETY: this thread holds the value shared.
		unsafe { &*self.object.value() }
	}
}

impl<T> Drop for Local<'_, T> {
	fn drop(&mut self) {
		self.object.release_local();
	}
}

pub struct Exclusive<'a, T> {
	object: &'a Object<T>,
}

impl<T> Deref for Exclusive<'_, T> {
	type Target = T;
	fn deref(&self) -> &T {
		// SAFETY: the exclusive borrow is held.
		unsafe { &*self.object.value() }
	}
}

impl<T> DerefMut for Exclusive<'_, T> {
	fn deref_mut(&mut self) -> &mut T {
		// SAFETY: the exclusive borrow is held.
		unsafe { &mut *self.object.value() }
	}
}

impl<T> Drop for Exclusive<'_, T> {
	fn drop(&mut self) {
		self.object.state.store(0, Ordering::Release);
	}
}

pub struct SharedArc<T> {
	object: Arc<Object<T>>,
}

impl<T> Deref for SharedArc<T> {
	type Target = T;
	fn deref(&self) -> &T {
		// SAFETY: a shared borrow is held.
		unsafe { &*self.object.value() }
	}
}

impl<T> Drop for SharedArc<T> {
	fn drop(&mut self) {
		self.object.release();
	}
}

pub struct ExclusiveArc<T> {
	object: Arc<Object<T>>,
}

impl<T> Deref for ExclusiveArc<T> {
	type Target = T;
	fn deref(&self) -> &T {
		// SAFETY: the exclusive borrow is held.
		unsafe { &*self.object.value() }
	}
}

impl<T> DerefMut for ExclusiveArc<T> {
	fn deref_mut(&mut self) -> &mut T {
		// SAFETY: the exclusive borrow is held.
		unsafe { &mut *self.object.value() }
	}
}

impl<T> Drop for ExclusiveArc<T> {
	fn drop(&mut self) {
		self.object.state.store(0, Ordering::Release);
	}
}

/// A foreign waker: `wake` may be called from any thread, any number of times, until
/// `drop` releases `data`. Either function may be null.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Waker {
	pub wake: Option<unsafe extern "C" fn(*mut c_void)>,
	pub drop: Option<unsafe extern "C" fn(*mut c_void)>,
	pub data: *mut c_void,
}

impl Waker {
	/// Whether `other` is this waker again, which a poll reuses rather than wrapping anew.
	fn same(&self, other: &Self) -> bool {
		self.wake.map(|f| f as usize) == other.wake.map(|f| f as usize) && self.data == other.data
	}

	/// The Rust waker around it, which calls `drop` once its last clone drops. One Rust
	/// returned stays wrapped too, so its `data` names it for as long as a future may
	/// compare it.
	pub fn lift(self) -> std::task::Waker {
		match (self.wake, self.drop) {
			// Nothing to wake or release: a ready future's first poll takes no allocation.
			(None, None) => std::task::Waker::noop().clone(),
			_ => std::task::Waker::from(Arc::new(Foreign(self))),
		}
	}

	/// Wake it, as C does.
	///
	/// # Safety
	/// It is not yet dropped.
	pub unsafe fn wake(&self) {
		if let Some(wake) = self.wake {
			// SAFETY: the waker's contract.
			unsafe { wake(self.data) };
		}
	}

	/// Drop it, as C does once.
	///
	/// # Safety
	/// It is not yet dropped, and is never used again.
	pub unsafe fn release(self) {
		if let Some(drop) = self.drop {
			// SAFETY: the waker's contract.
			unsafe { drop(self.data) };
		}
	}

	/// Rust's waker, or a context's, as one C owns: `wake` wakes it, and `drop` releases it.
	pub fn lower(waker: impl Into<Returned>) -> Self {
		Self {
			wake: Some(rust_wake),
			drop: Some(rust_drop),
			data: Box::into_raw(Box::new(waker.into().0)).cast(),
		}
	}
}

/// A waker a function returns: its own, or a context's.
pub struct Returned(std::task::Waker);

impl From<std::task::Waker> for Returned {
	fn from(waker: std::task::Waker) -> Self {
		Self(waker)
	}
}

impl From<Context<'_>> for Returned {
	fn from(cx: Context<'_>) -> Self {
		Self(cx.waker().clone())
	}
}

unsafe extern "C" fn rust_wake(data: *mut c_void) {
	// SAFETY: `Waker::lower` made `data`, which C has not dropped.
	unsafe { &*data.cast::<std::task::Waker>() }.wake_by_ref();
}

unsafe extern "C" fn rust_drop(data: *mut c_void) {
	// SAFETY: `Waker::lower` made `data`, which C drops once.
	drop(unsafe { Box::from_raw(data.cast::<std::task::Waker>()) });
}

/// A future C implements, of `()`: Rust polls it with a waker of its own each time, from
/// any thread, until `poll` returns true, and calls `free` once it drops the future, done
/// or not. `free` may be null.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ForeignFuture {
	pub poll: Option<unsafe extern "C" fn(*mut c_void, Waker) -> bool>,
	pub free: Option<unsafe extern "C" fn(*mut c_void)>,
	pub data: *mut c_void,
}

impl ForeignFuture {
	/// The Rust future around it, which frees it when dropped.
	pub fn lift(self, what: &str) -> Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
		let Some(poll) = self.poll else {
			fail(what, "a future's poll is null");
		};
		Box::pin(Polled {
			poll,
			free: self.free,
			data: self.data,
		})
	}
}

/// A [`ForeignFuture`] Rust holds, whose poll is not null.
struct Polled {
	poll: unsafe extern "C" fn(*mut c_void, Waker) -> bool,
	free: Option<unsafe extern "C" fn(*mut c_void)>,
	data: *mut c_void,
}

// SAFETY: the foreign future's contract: it may be polled and freed from any thread.
unsafe impl Send for Polled {}

impl std::future::Future for Polled {
	type Output = ();

	fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> std::task::Poll<()> {
		// SAFETY: the foreign future's contract; C owns the waker it is given.
		match unsafe { (self.poll)(self.data, Waker::lower(cx.waker().clone())) } {
			true => std::task::Poll::Ready(()),
			false => std::task::Poll::Pending,
		}
	}
}

impl Drop for Polled {
	fn drop(&mut self) {
		if let Some(free) = self.free {
			// SAFETY: the foreign future's contract; Rust drops it once.
			unsafe { free(self.data) };
		}
	}
}

struct Foreign(Waker);

// SAFETY: the waker's contract: `wake` and `drop` may be called from any thread.
unsafe impl Send for Foreign {}
// SAFETY: as above.
unsafe impl Sync for Foreign {}

impl Wake for Foreign {
	fn wake(self: Arc<Self>) {
		self.wake_by_ref();
	}

	fn wake_by_ref(self: &Arc<Self>) {
		if let Some(wake) = self.0.wake {
			// SAFETY: the waker's contract.
			unsafe { wake(self.0.data) };
		}
	}
}

impl Drop for Foreign {
	fn drop(&mut self) {
		if let Some(drop) = self.0.drop {
			// SAFETY: the waker's contract; this is the last reference.
			unsafe { drop(self.0.data) };
		}
	}
}

/// The data of a function C lends for a call, which Rust only passes back to it.
#[derive(Clone, Copy)]
pub struct Data(*mut c_void);

// SAFETY: the lent function's contract: it may be called from any thread.
unsafe impl Send for Data {}
// SAFETY: as above.
unsafe impl Sync for Data {}

impl Data {
	pub fn new(data: *mut c_void) -> Self {
		Self(data)
	}

	/// The pointer, through a method so a closure captures `Data` whole, not its field.
	pub fn get(self) -> *mut c_void {
		self.0
	}
}

/// The data of a function C implements that Rust keeps, which it frees once dropped.
pub struct Kept {
	data: *mut c_void,
	free: Option<unsafe extern "C" fn(*mut c_void)>,
}

// SAFETY: the kept function's contract: it, and its free, may be called from any thread.
unsafe impl Send for Kept {}
// SAFETY: as above.
unsafe impl Sync for Kept {}

impl Kept {
	pub fn new(data: *mut c_void, free: Option<unsafe extern "C" fn(*mut c_void)>) -> Self {
		Self { data, free }
	}

	pub fn get(&self) -> *mut c_void {
		self.data
	}
}

impl Drop for Kept {
	fn drop(&mut self) {
		if let Some(free) = self.free {
			// SAFETY: the kept function's contract; Rust drops it once.
			unsafe { free(self.data) };
		}
	}
}

/// A reader C implements, which Rust reads and seeks during the call it is lent to.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ForeignReader {
	pub read: Option<unsafe extern "C" fn(*mut c_void, *mut u8, usize, *mut usize, *mut Buf) -> bool>,
	pub seek: Option<unsafe extern "C" fn(*mut c_void, std::ffi::c_int, i64, *mut u64, *mut Buf) -> bool>,
	pub data: *mut c_void,
}

impl ForeignReader {
	/// The Rust reader around it.
	pub fn lift(self, what: &'static str) -> Reader {
		let (Some(read), Some(seek)) = (self.read, self.seek) else {
			fail(what, "a reader's function is null");
		};
		Reader {
			read,
			seek,
			data: self.data,
			what,
		}
	}
}

/// A [`ForeignReader`] whose functions are not null.
pub struct Reader {
	read: unsafe extern "C" fn(*mut c_void, *mut u8, usize, *mut usize, *mut Buf) -> bool,
	seek: unsafe extern "C" fn(*mut c_void, std::ffi::c_int, i64, *mut u64, *mut Buf) -> bool,
	data: *mut c_void,
	what: &'static str,
}

// SAFETY: the reader's contract: it may be called from any thread, one call at a time.
unsafe impl Send for Reader {}
// SAFETY: as above; every call takes `&mut self`.
unsafe impl Sync for Reader {}

impl Reader {
	/// The error a failed call wrote, as its message.
	fn error(&self, err: Buf) -> std::io::Error {
		// SAFETY: the reader's contract: a buffer `<prefix>_buf_copy` made.
		std::io::Error::other(unsafe { err.take_string(self.what) })
	}
}

impl std::io::Read for Reader {
	fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
		let mut n = 0;
		let mut err = Buf::NONE;
		// SAFETY: the reader's contract.
		match unsafe { (self.read)(self.data, buf.as_mut_ptr(), buf.len(), &mut n, &mut err) } {
			true if n <= buf.len() => Ok(n),
			true => fail(self.what, "a reader read more than it was asked"),
			false => Err(self.error(err)),
		}
	}
}

impl std::io::Seek for Reader {
	fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
		// C's `whence`, which every platform numbers alike.
		let (whence, offset) = match pos {
			// Its bits, which the reader reads back unsigned, so a start past `i64::MAX`
			// reaches it as Rust's own readers such as `Cursor` take one.
			std::io::SeekFrom::Start(offset) => (0, offset.cast_signed()),
			std::io::SeekFrom::Current(offset) => (1, offset),
			std::io::SeekFrom::End(offset) => (2, offset),
		};
		let mut position = 0;
		let mut err = Buf::NONE;
		// SAFETY: the reader's contract.
		match unsafe { (self.seek)(self.data, whence, offset, &mut position, &mut err) } {
			true => Ok(position),
			false => Err(self.error(err)),
		}
	}
}

pub const PENDING: i32 = 0;
pub const READY: i32 = 1;
pub const ERROR: i32 = 2;

/// A future behind a handle, polled from C: the head of the one allocation holding the
/// future and the function that lowers its result, which [`Future::new`] makes.
#[repr(C)]
pub struct Future {
	/// Poll the allocation, lowering the result through the poll's out pointers once ready.
	poll: unsafe fn(*mut Future, Waker, &[*mut c_void], *mut c_void, &str) -> i32,
	/// Drop the allocation.
	drop: unsafe fn(*mut Future),
	/// The last foreign waker, and the Rust waker around it.
	waker: Option<(Waker, std::task::Waker)>,
	polling: AtomicBool,
}

/// The allocation behind a handle: its head first, so the handle points at both.
#[repr(C)]
struct Typed<F, L> {
	head: Future,
	/// The future, pinned here until it finishes and drops in place.
	future: Option<F>,
	/// Writes the finished future's result through the poll's out pointers, one per value
	/// it returns, and its error pointer, returning the poll. The poll that finishes the
	/// future calls it, so it never crosses threads.
	lower: L,
}

impl Future {
	pub fn new<F, L>(future: F, lower: L) -> *mut Self
	where
		F: std::future::Future + Send + 'static,
		// Send, as any poll may finish the future on another thread.
		L: Fn(F::Output, &[*mut c_void], *mut c_void) -> i32 + Send + 'static,
	{
		let typed = Box::new(Typed {
			head: Self {
				poll: poll_typed::<F, L>,
				drop: drop_typed::<F, L>,
				waker: None,
				polling: AtomicBool::new(false),
			},
			future: Some(future),
			lower,
		});
		Box::into_raw(typed).cast()
	}

	/// Poll it once with `waker`, writing the result through `outs` and `err` when ready.
	///
	/// # Safety
	/// `future` is a live future handle, and each of `outs` and `err` is null or points
	/// where the function's result types go.
	pub unsafe fn poll(future: *mut Self, waker: Waker, outs: &[*mut c_void], err: *mut c_void, what: &str) -> i32 {
		if future.is_null() {
			fail(what, "a future is null");
		}
		// SAFETY: the caller's contract; only the flag is touched before it is held.
		let head = unsafe { &*future };
		if head.polling.swap(true, Ordering::Acquire) {
			fail(what, "a future is polled concurrently");
		}
		// SAFETY: the flag makes this the only access, and `new` made the allocation.
		let result = unsafe { (head.poll)(future, waker, outs, err, what) };
		// SAFETY: as above; the allocation outlives the poll.
		unsafe { &*future }.polling.store(false, Ordering::Release);
		result
	}

	/// Make the future's first poll, with no waker: on `READY` or `ERROR` the result is
	/// written, the future freed, and null written through `handle`; on `PENDING` its
	/// handle goes there, for C to poll with a waker and free.
	///
	/// # Safety
	/// `future` is a handle `new` just made, `handle` is null, which aborts, or points
	/// where a handle goes, and each of `outs` and `err` is null or points where the function's result
	/// types go.
	pub unsafe fn start(
		future: *mut Self,
		outs: &[*mut c_void],
		err: *mut c_void,
		handle: *mut *mut Self,
		what: &str,
	) -> i32 {
		if handle.is_null() {
			fail(what, "a future's handle pointer is null");
		}
		let none = Waker {
			wake: None,
			drop: None,
			data: std::ptr::null_mut(),
		};
		// SAFETY: the caller's contract.
		let result = unsafe { Self::poll(future, none, outs, err, what) };
		match result {
			// SAFETY: the caller's contract.
			PENDING => unsafe { handle.write(future) },
			// SAFETY: it finished; null tells C there is no handle, and `free` takes it.
			_ => unsafe {
				handle.write(std::ptr::null_mut());
				Self::free(future)
			},
		}
		result
	}

	/// Drop the future, cancelling it if it has not finished.
	///
	/// # Safety
	/// `future` is null or a live future handle, freed once and not being polled.
	pub unsafe fn free(future: *mut Self) {
		if !future.is_null() {
			// SAFETY: the caller's contract.
			unsafe { ((*future).drop)(future) };
		}
	}
}

/// [`Future`]'s poll of a `Typed<F, L>`.
///
/// # Safety
/// `future` is a live `Typed<F, L>` that only this call accesses, and each of `outs` and
/// `err` is null or points where the function's result types go.
unsafe fn poll_typed<F: std::future::Future, L: Fn(F::Output, &[*mut c_void], *mut c_void) -> i32>(
	future: *mut Future,
	waker: Waker,
	outs: &[*mut c_void],
	err: *mut c_void,
	what: &str,
) -> i32 {
	// SAFETY: the caller's contract.
	let Typed { head, future, lower } = unsafe { &mut *future.cast::<Typed<F, L>>() };
	let Some(inner) = future.as_mut() else {
		fail(what, "a future is polled after it finished");
	};
	if !head.waker.as_ref().is_some_and(|(last, _)| last.same(&waker)) {
		head.waker = Some((waker, waker.lift()));
	}
	let (_, rust) = head.waker.as_ref().expect("set above");
	let mut cx = Context::from_waker(rust);
	// SAFETY: the allocation never moves, and the future drops in place.
	match unsafe { Pin::new_unchecked(inner) }.poll(&mut cx) {
		std::task::Poll::Pending => PENDING,
		std::task::Poll::Ready(value) => {
			*future = None;
			head.waker = None;
			lower(value, outs, err)
		}
	}
}

/// [`Future`]'s drop of a `Typed<F, L>`.
///
/// # Safety
/// `future` is a live `Typed<F, L>`, dropped once.
unsafe fn drop_typed<F, L>(future: *mut Future) {
	// SAFETY: the caller's contract; `new` boxed it.
	drop(unsafe { Box::from_raw(future.cast::<Typed<F, L>>()) });
}

/// Write `value` through `out` if the caller wants it, or drop it.
///
/// # Safety
/// `out` is null or points where a `T` goes.
pub unsafe fn write<T>(out: *mut c_void, value: impl FnOnce() -> T) {
	if !out.is_null() {
		// SAFETY: the caller's contract.
		unsafe { out.cast::<T>().write(value()) };
	}
}
