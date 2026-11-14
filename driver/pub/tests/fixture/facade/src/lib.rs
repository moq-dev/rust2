//! A library nothing in the workspace depends on, whose API crates outside it may use,
//! as a translator's fixture's is.

/// External: this library's API, re-exported from api.
pub use api::Exposed;

/// External: this library's API, though nothing calls it.
pub fn unused() -> api::Returned {
	api::Returned::default()
}

/// External: this library's API, re-exported from api, with what its items return.
pub use api::Shape;

/// External: this library's API, which exposes all of extra's.
pub extern crate extra;

/// External: this library's API, re-exported from api through a glob there.
pub use api::Part;
