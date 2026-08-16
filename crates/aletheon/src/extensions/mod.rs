//! Aletheon-owned extension lifecycle and runtime composition.
//!
//! Production Aletheon wiring uses this crate so package install, activation,
//! snapshot publication, and runtime routing do not enter the Aletheon
//! composition/application facade.

pub mod extension_coordinator;
pub mod extension_install;
pub mod extension_manage;
pub mod extension_runtime_router;
pub mod extension_service;
pub mod extension_snapshot;
pub mod gmail;
pub mod subprocess;

pub use extension_coordinator::{ExtensionCoordinator, ExtensionRuntimePublisher};
pub use extension_install::ExtensionInstallService;
pub use extension_manage::{
    DenyPermissionElevation, ExplicitOperatorApproval, ExtensionApprovalDecision,
    ExtensionApprovalPort, ExtensionApprovalRequest, ExtensionDoctorResult, ExtensionManageService,
};
pub use extension_service::{
    ActivatedExtensions, ExtensionActivationDecision, ExtensionDecisionSink, ExtensionService,
    NoopExtensionDecisionSink, SessionExtensionPolicy, SpineExtensionDecisionSink,
};
pub use extension_snapshot::{
    ExtensionAgentProfileAsset, ExtensionConnectorAsset, ExtensionRuntimeSnapshot,
    ExtensionRuntimeView, ExtensionSnapshotCompiler,
};
