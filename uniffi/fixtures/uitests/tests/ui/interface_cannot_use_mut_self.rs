
// The path is relative to this file.
uniffi_macros::generate_and_include_scaffolding!("../../src/counter.udl");

fn main() { /* empty main required by `trybuild` */}

pub struct Counter {
    value: u32,
}

impl Counter {
    pub fn new() -> Self {
        Self { value: 0 }
    }

    // This will fail to compile due to `&mut self` receiver.
    pub fn increment(&mut self) -> u32 {
        self.value = self.value + 1;
        self.value
    }
}

