// The path is relative to this file.
uniffi_macros::generate_and_include_scaffolding!("../../src/trait.udl");

fn main() { /* empty main required by `trybuild` */}

// This will fail to compile, because the trait is not explicit Send+Sync
#[uniffi::trait_interface]
pub trait Trait {
}

// This will fail to compile, because the trait is not explicit Send+Sync
#[uniffi::export]
pub trait ProcMacroTrait {
}
