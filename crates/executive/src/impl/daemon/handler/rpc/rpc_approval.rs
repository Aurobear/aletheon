//! Durable approval JSON-RPC handlers.
//!
//! These methods are deliberately separate from `approval_response`, which
//! resolves the in-memory one-shot gate used by synchronous tool execution.

use super::RequestHandler;
use crate::r#impl::approval::{
    ApprovalDecision, ApprovalRepository, ApprovalRepositoryError, ApprovalResolutionContext,
};
use fabric::{ApprovalId, PrincipalId};
use serde_json::{json, Value};

const INVALID_PARAMS: i64 = -32602;
const APPROVAL_NOT_FOUND: i64 = -32041;
const APPROVAL_FORBIDDEN: i64 = -32043;
const APPROVAL_CONFLICT: i64 = -32049;
const APPROVAL_STORAGE: i64 = -32040;

#[derive(Clone, Debug)]
struct AuthenticatedApprovalContext {
    principal_id: PrincipalId,
    channel: String,
}

impl RequestHandler {
    async fn authenticated_approval_context(
        &self,
    ) -> Result<AuthenticatedApprovalContext, anyhow::Error> {
        // The session gateway establishes this identity. Request JSON is not
        // consulted, so callers cannot select another approval owner/channel.
        let session_id = self.get_or_create_session(None).await?.0;
        Ok(AuthenticatedApprovalContext {
            principal_id: PrincipalId(session_id),
            channel: "local_rpc".into(),
        })
    }

    pub(super) async fn handle_approval_list(&self, id: &Value, _request: &Value) -> Value {
        let context = match self.authenticated_approval_context().await {
            Ok(value) => value,
            Err(error) => return rpc_error(id, APPROVAL_STORAGE, error.to_string()),
        };
        let now_ms = self.subsystems.ports.clock.wall_now().0;
        let repository = self.subsystems.memory.approval_repository.lock().await;
        match list(&repository, &context, now_ms) {
            Ok(approvals) => json!({"jsonrpc":"2.0", "id":id, "result":{"approvals":approvals}}),
            Err(error) => repository_error(id, error),
        }
    }

    pub(super) async fn handle_approval_show(&self, id: &Value, request: &Value) -> Value {
        let context = match self.authenticated_approval_context().await {
            Ok(value) => value,
            Err(error) => return rpc_error(id, APPROVAL_STORAGE, error.to_string()),
        };
        let approval_id = match parse_id(request) {
            Ok(value) => value,
            Err(message) => return rpc_error(id, INVALID_PARAMS, message),
        };
        let repository = self.subsystems.memory.approval_repository.lock().await;
        match show(&repository, &context, approval_id) {
            Ok(approval) => json!({"jsonrpc":"2.0", "id":id, "result":{"approval":approval}}),
            Err(error) => repository_error(id, error),
        }
    }

    pub(super) async fn handle_approval_approve(&self, id: &Value, request: &Value) -> Value {
        self.handle_durable_resolution(id, request, ApprovalDecision::Approve)
            .await
    }

    pub(super) async fn handle_approval_reject(&self, id: &Value, request: &Value) -> Value {
        let reason = request["params"]["reason"].as_str().map(str::to_owned);
        self.handle_durable_resolution(id, request, ApprovalDecision::Reject { reason })
            .await
    }

    async fn handle_durable_resolution(
        &self,
        id: &Value,
        request: &Value,
        decision: ApprovalDecision,
    ) -> Value {
        let context = match self.authenticated_approval_context().await {
            Ok(value) => value,
            Err(error) => return rpc_error(id, APPROVAL_STORAGE, error.to_string()),
        };
        let approval_id = match parse_id(request) {
            Ok(value) => value,
            Err(message) => return rpc_error(id, INVALID_PARAMS, message),
        };
        let version = match request["params"]["version"].as_u64() {
            Some(value) => value,
            None => return rpc_error(id, INVALID_PARAMS, "version must be an unsigned integer"),
        };
        let now_ms = self.subsystems.ports.clock.wall_now().0;
        let repository = self.subsystems.memory.approval_repository.lock().await;
        match resolve(
            &repository,
            &context,
            approval_id,
            version,
            decision,
            now_ms,
        ) {
            Ok(approval) => json!({"jsonrpc":"2.0", "id":id, "result":{"approval":approval}}),
            Err(error) => repository_error(id, error),
        }
    }
}

fn parse_id(request: &Value) -> Result<ApprovalId, &'static str> {
    let raw = request["params"]["approval_id"]
        .as_str()
        .ok_or("approval_id must be a UUID string")?;
    uuid::Uuid::parse_str(raw)
        .map(ApprovalId)
        .map_err(|_| "approval_id must be a UUID string")
}

fn list(
    repository: &ApprovalRepository,
    context: &AuthenticatedApprovalContext,
    now_ms: i64,
) -> Result<Vec<fabric::ApprovalSnapshot>, ApprovalRepositoryError> {
    repository.list_pending(&context.principal_id, now_ms)
}

fn show(
    repository: &ApprovalRepository,
    context: &AuthenticatedApprovalContext,
    id: ApprovalId,
) -> Result<fabric::ApprovalSnapshot, ApprovalRepositoryError> {
    let approval = repository
        .get(id)?
        .ok_or(ApprovalRepositoryError::NotFound(id))?;
    if approval.owner_id != context.principal_id {
        return Err(ApprovalRepositoryError::WrongOwner);
    }
    Ok(approval)
}

fn resolve(
    repository: &ApprovalRepository,
    context: &AuthenticatedApprovalContext,
    id: ApprovalId,
    version: u64,
    decision: ApprovalDecision,
    now_ms: i64,
) -> Result<fabric::ApprovalSnapshot, ApprovalRepositoryError> {
    repository.resolve(
        id,
        version,
        &ApprovalResolutionContext {
            principal_id: context.principal_id.clone(),
            channel: context.channel.clone(),
        },
        decision,
        now_ms,
    )
}

fn repository_error(id: &Value, error: ApprovalRepositoryError) -> Value {
    let code = match error {
        ApprovalRepositoryError::NotFound(_) => APPROVAL_NOT_FOUND,
        ApprovalRepositoryError::WrongOwner | ApprovalRepositoryError::ChannelDenied => {
            APPROVAL_FORBIDDEN
        }
        ApprovalRepositoryError::AlreadyDecided
        | ApprovalRepositoryError::VersionConflict { .. }
        | ApprovalRepositoryError::ActiveSubjectConflict => APPROVAL_CONFLICT,
        _ => APPROVAL_STORAGE,
    };
    rpc_error(id, code, error.to_string())
}

fn rpc_error(id: &Value, code: i64, message: impl Into<String>) -> Value {
    json!({"jsonrpc":"2.0", "id":id, "error":{"code":code, "message":message.into()}})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::r#impl::approval::ApprovalCreate;
    use crate::r#impl::goal::ObjectiveStore;
    use fabric::{
        ApprovalCategory, ApprovalRisk, ApprovalSubject, GoalBudget, GoalSpec, GoalState,
    };
    use tempfile::NamedTempFile;

    struct Fixture {
        _file: NamedTempFile,
        repository: ApprovalRepository,
        owner: AuthenticatedApprovalContext,
        other: AuthenticatedApprovalContext,
        approval: fabric::ApprovalSnapshot,
    }

    impl Fixture {
        fn new() -> Self {
            let file = NamedTempFile::new().unwrap();
            let store = ObjectiveStore::open(file.path()).unwrap();
            let owner_id = PrincipalId("authenticated-session".into());
            let goal = store
                .create_goal(
                    &owner_id,
                    "authenticated-session",
                    "session",
                    &GoalSpec {
                        original_intent: "approve change".into(),
                        desired_state: vec![],
                        constraints: vec![],
                        acceptance_criteria: vec![],
                        budget: GoalBudget::default(),
                    },
                )
                .unwrap();
            let goal = store
                .transition_goal(goal.id, goal.version, GoalState::Running, None, &json!({}))
                .unwrap();
            drop(store);
            let repository = ApprovalRepository::open(file.path()).unwrap();
            let approval = repository
                .create(ApprovalCreate {
                    subject: ApprovalSubject {
                        category: ApprovalCategory::ApplyCode,
                        goal_id: goal.id,
                        attempt_id: None,
                        job_id: None,
                        attributes: Default::default(),
                        allowed_scope: vec![],
                        apply_target: None,
                    },
                    risk: ApprovalRisk::High,
                    summary: "verified diff".into(),
                    artifacts: vec![],
                    created_at_ms: 10,
                    expires_at_ms: 1_000,
                })
                .unwrap();
            Self {
                _file: file,
                repository,
                owner: AuthenticatedApprovalContext {
                    principal_id: owner_id,
                    channel: "local_rpc".into(),
                },
                other: AuthenticatedApprovalContext {
                    principal_id: PrincipalId("forged-json-owner".into()),
                    channel: "local_rpc".into(),
                },
                approval,
            }
        }
    }

    #[test]
    fn list_and_show_are_bound_to_authenticated_principal() {
        let f = Fixture::new();
        assert_eq!(list(&f.repository, &f.owner, 20).unwrap().len(), 1);
        assert!(list(&f.repository, &f.other, 20).unwrap().is_empty());
        assert_eq!(
            show(&f.repository, &f.owner, f.approval.id).unwrap().id,
            f.approval.id
        );
        assert!(matches!(
            show(&f.repository, &f.other, f.approval.id),
            Err(ApprovalRepositoryError::WrongOwner)
        ));
    }

    #[test]
    fn approve_and_reject_use_authenticated_context_and_version() {
        let f = Fixture::new();
        assert!(matches!(
            resolve(
                &f.repository,
                &f.other,
                f.approval.id,
                0,
                ApprovalDecision::Approve,
                20,
            ),
            Err(ApprovalRepositoryError::WrongOwner)
        ));
        let approved = resolve(
            &f.repository,
            &f.owner,
            f.approval.id,
            0,
            ApprovalDecision::Approve,
            20,
        )
        .unwrap();
        assert_eq!(approved.status, fabric::ApprovalStatus::Approved);
    }

    #[test]
    fn transient_and_durable_namespaces_do_not_overlap() {
        let f = Fixture::new();
        // A transient gate ID is opaque text and cannot enter the durable UUID namespace.
        let transient_request = json!({"params":{"approval_id":"tool-gate-42"}});
        assert!(parse_id(&transient_request).is_err());

        // A UUID unknown to the durable repository cannot resolve any durable request.
        let transient_uuid = ApprovalId::new();
        assert!(matches!(
            resolve(
                &f.repository,
                &f.owner,
                transient_uuid,
                0,
                ApprovalDecision::Approve,
                20,
            ),
            Err(ApprovalRepositoryError::NotFound(_))
        ));
        assert_eq!(
            f.repository.get(f.approval.id).unwrap().unwrap().status,
            fabric::ApprovalStatus::Pending
        );
    }
}
