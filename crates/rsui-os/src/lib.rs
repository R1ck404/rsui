//! What rsui asks of the operating system below the window, written once for each system so the crates above don't
//! each carry their own copy: memory by the page ([`vm`]) and stacks ([`stack`]).
//!
//! On Windows the crates that were written first still call the system themselves, as they always have, and nothing
//! here changes what they compile to. Elsewhere (Linux, Android, macOS) they come here. Each function is small and
//! inlined, so going through this crate costs nothing over calling the system directly.

#![deny(unsafe_op_in_unsafe_fn)]

#[cfg(unix)]
pub mod map;
pub mod stack;
pub mod vm;
