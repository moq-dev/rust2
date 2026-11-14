//! A crate the api fixture names in signatures but re-exports nothing from, as moq-tokio's
//! listener names rustls's server config and moq-net's `dial_tcp` names kio's `Pending`.

/// A config: an opaque handle wherever the api fixture returns or takes an `Arc` of it.
#[derive(Debug)]
pub struct Config {
	alpn: String,
}

impl Config {
	pub fn new(alpn: &str) -> Self {
		Self { alpn: alpn.to_string() }
	}

	pub fn alpn(&self) -> &str {
		&self.alpn
	}
}

mod session {
	use crate::policy::Lifetime;

	/// A session: an object of a private module, which the api fixture returns bare and no
	/// re-export names, so it exports by its path in this crate.
	pub struct Session {
		alpn: String,
		lifetime: Lifetime,
	}

	impl Session {
		pub fn alpn(&self) -> String {
			self.alpn.clone()
		}

		/// A record only this method names, which exports once the session does.
		pub fn lifetime(&self) -> Lifetime {
			self.lifetime
		}
	}

	/// A session of `alpn` lasting `seconds`, or an error when `alpn` is empty.
	pub fn resume(alpn: &str, seconds: u32) -> Result<Session, crate::Error> {
		if alpn.is_empty() {
			return Err(crate::Error("no protocol to resume".to_string()));
		}
		Ok(Session {
			alpn: alpn.to_string(),
			lifetime: Lifetime { seconds },
		})
	}
}

pub use session::{Session, resume};

pub mod policy {
	/// How long a session lasts.
	#[derive(Clone, Copy, Debug, PartialEq)]
	pub struct Lifetime {
		pub seconds: u32,
	}
}

/// An error, which the api fixture returns only as a `Result`'s, so it crosses as its
/// message.
#[derive(Debug)]
pub struct Error(String);

impl std::fmt::Display for Error {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str(&self.0)
	}
}

/// Types with no public path in this crate, as rustls's are where the facade names a
/// module `rustls` of its own: nothing exports them with their API, so an export that
/// names one by value holds it in an opaque handle.
#[doc(hidden)]
pub mod held {
	/// A certificate: an opaque handle wherever the api fixture takes or returns one by
	/// value, as moq-tokio's TLS listener loads rustls's root certificates.
	#[derive(Clone, Debug)]
	pub struct Cert {
		der: Vec<u8>,
	}

	impl Cert {
		pub fn new(der: &[u8]) -> Self {
			Self { der: der.to_vec() }
		}

		pub fn der(&self) -> &[u8] {
			&self.der
		}
	}

	/// A session handed over from elsewhere, which no export of the api fixture returns: C
	/// can never hold one.
	#[derive(Debug)]
	pub struct Handoff {
		pub id: u32,
	}

	/// Why a certificate is refused, which the api fixture's `verify` returns as its message
	/// and `blame` takes back, though no export returns one: C never holds one.
	#[derive(Debug)]
	pub struct Invalid {
		pub reason: String,
	}

	impl std::fmt::Display for Invalid {
		fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
			write!(f, "invalid certificate: {}", self.reason)
		}
	}

	impl std::error::Error for Invalid {}

	/// Why text is not a certificate, borrowing the text: an error that crosses as its
	/// message, so the borrow ends with the call.
	#[derive(Debug)]
	pub struct Malformed<'a>(pub &'a str);

	impl std::fmt::Display for Malformed<'_> {
		fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
			write!(f, "malformed: {}", self.0)
		}
	}

	impl std::error::Error for Malformed<'_> {}

	/// What a poll registers its waker with, made from the waker alone, as kio's `Waiter` is.
	pub struct Waiter(std::task::Waker);

	impl Waiter {
		pub fn new(waker: std::task::Waker) -> Self {
			Self(waker)
		}

		pub fn wake(&self) {
			self.0.wake_by_ref();
		}
	}

	/// A count that is not `Send`, which the reject fixture takes by value: no handle may hold it.
	pub struct Local(std::rc::Rc<u32>);

	impl Local {
		pub fn get(&self) -> u32 {
			*self.0
		}
	}

	/// An error that borrows and takes a type parameter, so no handle holds it: it crosses as
	/// its message.
	#[derive(Debug)]
	pub struct Wrapped<'a, T>(pub &'a str, pub T);

	impl<T> std::fmt::Display for Wrapped<'_, T> {
		fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
			f.write_str(self.0)
		}
	}

	/// A future that is not an `impl Future`, resolving to what it borrows; its type
	/// parameter keeps any handle from holding it.
	pub struct Lend<'a, T>(pub &'a str, pub std::marker::PhantomData<T>);

	impl<'a, T> std::future::Future for Lend<'a, T> {
		type Output = &'a str;

		fn poll(self: std::pin::Pin<&mut Self>, _: &mut std::task::Context<'_>) -> std::task::Poll<&'a str> {
			std::task::Poll::Ready(self.0)
		}
	}

	/// A future named by a trait of its own, whose borrowed output only its supertrait names.
	pub trait Borrowing<'a>: std::future::Future<Output = &'a str> {}

	impl<'a, F: std::future::Future<Output = &'a str>> Borrowing<'a> for F {}

	/// A future that is not an `impl Future`, borrowing what it reads but resolving to an
	/// owned count.
	pub struct Pending<'a>(pub &'a [u8]);

	impl std::future::Future for Pending<'_> {
		type Output = usize;

		fn poll(self: std::pin::Pin<&mut Self>, _: &mut std::task::Context<'_>) -> std::task::Poll<usize> {
			std::task::Poll::Ready(self.0.len())
		}
	}

	/// A `Copy` token, which an opaque handle holds by value.
	#[derive(Clone, Copy, Debug)]
	pub struct Token(pub u32);

	/// A trait C never implements, whose `Arc<dyn Resolver>` an opaque handle holds, as
	/// moq-tokio's TLS server takes rustls's certificate resolver.
	pub trait Resolver: Send + Sync {
		fn name(&self) -> String;
	}

	/// A resolver of a fixed name.
	pub struct Named(pub String);

	impl Resolver for Named {
		fn name(&self) -> String {
			self.0.clone()
		}
	}

	/// A trait whose `&'static dyn Codec` an opaque handle holds by value, as rustls's
	/// client config lists its certificate compressors.
	pub trait Codec: Send + Sync {
		fn id(&self) -> u8;
	}

	/// A codec of a fixed id.
	pub struct Fixed(pub u8);

	impl Codec for Fixed {
		fn id(&self) -> u8 {
			self.0
		}
	}

	/// The codecs there are, each `'static`.
	pub static ZLIB: Fixed = Fixed(1);
	pub static BROTLI: Fixed = Fixed(2);

	/// A stream no type C holds implements, which C cannot implement either, as tokio's
	/// `AsyncRead` bounds the stream moq-tokio's TLS acceptor takes.
	pub trait Stream {
		fn read(&mut self) -> u8;
	}

	/// A sink every reference meets, whatever it refers to, so a value C holds may meet it.
	pub trait Sink {
		fn write(&self, byte: u8) -> u8;
	}

	impl<T> Sink for &T {
		fn write(&self, byte: u8) -> u8 {
			byte
		}
	}

	/// A width only `u32` has, a value C passes by role, so a value C holds may meet it.
	pub trait Width {
		fn bits(&self) -> u32;
	}

	impl Width for u32 {
		fn bits(&self) -> u32 {
			32
		}
	}
}
