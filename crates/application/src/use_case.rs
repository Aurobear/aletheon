//! Marker types for the narrow Session use-case vocabulary.

/// Create a session by forwarding a typed `runtime::CreateSessionCommand`.
pub struct CreateSession;

/// Resume a session from a reference string (opaque; Runtime resolves it).
pub struct ResumeSession;

/// Fork a session (Runtime assigns the child identity).
pub struct ForkSession;

/// List sessions.
pub struct ListSessions;

/// Get a single session snapshot.
pub struct GetSession;

/// Delete a session.
pub struct DeleteSession;
