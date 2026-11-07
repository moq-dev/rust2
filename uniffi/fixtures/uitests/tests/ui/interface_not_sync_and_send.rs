
use std::cell::Cell;

// The path is relative to this file.
uniffi_macros::generate_and_include_scaffolding!("../../src/counter.udl");

fn main() { /* empty main required by `trybuild` */}

pub struct Counter {
    // This will fail to compile, because `Cell` is not `Sync`.
    value: Cell<u32>,
}

impl Counter {
    pub fn new() -> Self {
        Self { value: Cell::new(0) }
    }

    pub fn increment(&self) -> u32 {
        let new_value = self.value.get() + 1;
        self.value.set(new_value);
        new_value
    }
}

#[derive(uniffi::Object)]
pub struct ProcMacroCounter {
    // This will fail to compile, because `Cell` is not `Sync`.
    value: Cell<u32>,
}

#[uniffi::export]
impl ProcMacroCounter {
    #[uniffi::constructor]
    pub fn new() -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self { value: Cell::new(0) })
    }

    pub fn increment(&self) -> u32 {
        let new_value = self.value.get() + 1;
        self.value.set(new_value);
        new_value
    }
}
