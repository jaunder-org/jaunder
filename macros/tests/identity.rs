//! The `#[client_only]` attribute must be a no-op: an annotated item compiles and
//! behaves exactly as if unannotated.

#[macros::client_only]
fn answer() -> u32 {
    42
}

#[macros::client_only]
fn add(a: u32, b: u32) -> u32 {
    a + b
}

#[test]
fn client_only_is_identity() {
    assert_eq!(answer(), 42);
    assert_eq!(add(2, 3), 5);
}
