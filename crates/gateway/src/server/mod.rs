//! Typed Gateway server boundary.
//!
//! The server owns versioned dispatch and Application error mapping.  It does
//! not open sockets or stores; transport binding and concrete use-case wiring
//! remain in the composition root.

pub mod handlers;
