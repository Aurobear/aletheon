use fixture_api_error_mapping::{domain::DomainError,wire::response};
#[test] fn maps_all_errors(){ assert_eq!(response(DomainError::Missing),(404,"missing")); assert_eq!(response(DomainError::Conflict),(409,"conflict")); assert_eq!(response(DomainError::Invalid),(422,"invalid")); }
