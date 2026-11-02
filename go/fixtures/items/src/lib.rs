//! Items whose Go names or identity take care: fields and variants whose escapes could
//! meet, and constant memory, which every use of shares.

/// Fields Go would name alike if escaping were not one to one.
struct Fields {
	r#type: u8,
	type_: u8,
	tag: u8,
	tag_: u8,
}

/// Variants Go would hold in one field if escaping were not one to one.
#[allow(non_camel_case_types, reason = "the names Go escapes")]
enum Kind {
	tag(u8),
	tag_(u8),
	r#type(u8),
}

fn byte() -> &'static u8 {
	&42
}

fn bytes() -> &'static [u8; 3] {
	b"abc"
}

/// Run the case the first byte picks on the rest.
pub fn run(input: &[u8]) -> Vec<u8> {
	let Some((&pick, data)) = input.split_first() else {
		return Vec::new();
	};
	let at = |i: usize| data.get(i).copied().unwrap_or(0);
	match pick % 2 {
		0 => {
			let f = Fields {
				r#type: at(0),
				type_: at(1),
				tag: at(2),
				tag_: at(3),
			};
			let kind = match at(4) % 3 {
				0 => Kind::tag(at(5)),
				1 => Kind::tag_(at(5)),
				_ => Kind::r#type(at(5)),
			};
			let (k, v) = match kind {
				Kind::tag(v) => (0, v),
				Kind::tag_(v) => (1, v),
				Kind::r#type(v) => (2, v),
			};
			vec![f.r#type, f.type_, f.tag, f.tag_, k, v]
		}
		_ => vec![std::ptr::eq(byte(), byte()) as u8, std::ptr::eq(bytes(), bytes()) as u8],
	}
}
