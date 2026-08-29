//! Synveda's HTTP application plane.
//!
//! The router authenticates callers, resolves the tenant and owned resource,
//! obtains a Cedar decision, enters tenant-scoped storage, and records the
//! action in the audit chain. Runtime writes append session events; runtime
//! reads create session-owned context runs. Governed mutations enter VedaFlow.
//! The generated OpenAPI document and executable route catalogue describe the
//! same router.
//!
//! This internal library exists so the gateway binary and behavior tests can
//! share assembly seams. Feature modules stay private unless one of those
//! consumers needs a narrow runtime or test contract.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod access;
mod admin_scopes;
pub mod app;
mod approvals;
mod audit;
mod audit_query;
mod auth;
pub mod authority;
pub mod authz;
mod capabilities;
mod capture;
mod channels;
mod configuration;
mod console;
mod context_api;
mod curators;
mod directory_admin;
pub mod directory_sync;
mod error;
mod idempotency;
pub mod knowledge;
mod knowledge_api;
mod knowledge_conflicts;
pub mod knowledge_index;
mod me;
pub mod okf;
pub mod openapi;
mod packs;
mod policy;
mod prompts;
mod proposals;
pub mod provision;
mod quarantine;
pub mod relaxations;
mod request;
mod response;
pub mod routes;
pub mod runtime_config;
mod scim;
mod service_identities;
mod sessions;
pub mod shutdown;
mod skills;
pub mod telemetry;
mod tenant;
mod tool_registry;
pub mod worker;
mod workspaces;
