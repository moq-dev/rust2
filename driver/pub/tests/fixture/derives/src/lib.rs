//! Macros that read their item's visibility, as any may, and one that does not.

use proc_macro::{TokenStream, TokenTree};

/// Kept: a proc-macro crate's macros run with nothing naming them.
#[proc_macro_derive(Label)]
pub fn derive_label(input: TokenStream) -> TokenStream {
	let (visibility, name) = read(input);
	format!("impl {name} {{ fn visibility() -> &'static str {{ {visibility:?} }} pub(crate) fn label() -> &'static str {{ Self::visibility() }} }}")
		.parse()
		.unwrap()
}

/// Kept: hands the visibility it reads to `make_label!`, a macro of the crate it expands
/// in.
#[proc_macro_derive(NestedLabel)]
pub fn derive_nested_label(input: TokenStream) -> TokenStream {
	let (visibility, name) = read(input);
	format!("make_label!({name}, {visibility:?});").parse().unwrap()
}

/// Kept: adds a function returning the visibility of the function it is on.
#[proc_macro_attribute]
pub fn tag(_: TokenStream, item: TokenStream) -> TokenStream {
	let (visibility, name) = read(item.clone());
	let tag: TokenStream = format!("fn {name}_tag() -> &'static str {{ {visibility:?} }}")
		.parse()
		.unwrap();
	item.into_iter().chain(tag).collect()
}

/// Kept: returns its item as it is.
#[proc_macro_attribute]
pub fn keep(_: TokenStream, item: TokenStream) -> TokenStream {
	item
}

/// The visibility of the struct, enum, or function `item`, and its name.
fn read(item: TokenStream) -> (String, String) {
	let mut visibility = String::new();
	let mut tokens = item.into_iter();
	while let Some(token) = tokens.next() {
		match &token {
			TokenTree::Ident(ident) if ["struct", "enum", "fn"].contains(&ident.to_string().as_str()) => {
				let name = tokens.next().expect("a name").to_string();
				return (visibility, name);
			}
			TokenTree::Punct(_) => {
				// An attribute: `#` and its bracketed group.
				tokens.next();
			}
			_ => visibility.push_str(&token.to_string()),
		}
	}
	panic!("a struct, enum, or function")
}
