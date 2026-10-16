//! Deployment custody selection shared by HTTP admission and durable workers.

use crate::{
    GitHubSourceCredential, GitHubSourceReader, GitHubSourceTarget, LocalSourceReader, SourcePin,
    SourceSnapshot,
};
use std::collections::BTreeMap;
use synveda_types::{Result, ScopeId, TenantId, TenantSecretId};

/// Optional inbound transports; a selected remote cannot fall back to local.
#[derive(Clone)]
pub struct SourceReaders {
    local: Option<LocalSourceReader>,
    github: Option<GitHubSourceReader>,
}
impl SourceReaders {
    /// Assemble validated deployment readers without granting product authority.
    #[must_use]
    pub fn new(local: Option<LocalSourceReader>, github: Option<GitHubSourceReader>) -> Self {
        Self { local, github }
    }
    /// Select exact scoped custody and bind a remote to the actual attachment.
    pub fn select(
        &self,
        tenant: TenantId,
        scope: ScopeId,
        target: &str,
        repository_uri: &str,
    ) -> Result<SourceReader<'_>> {
        crate::validate_target(target)?;
        if let Some(github) = &self.github {
            if let Some(mapping) = github.targets.iter().find(|mapping| {
                mapping.tenant_id == tenant && mapping.scope_id == scope && mapping.target == target
            }) {
                if !mapping
                    .repository_uri()
                    .eq_ignore_ascii_case(repository_uri)
                {
                    return Err(crate::source_github::unavailable());
                }
                return Ok(SourceReader {
                    kind: Kind::GitHub(github, mapping),
                    tenant,
                    scope,
                    target: target.to_owned(),
                });
            }
            if github
                .targets
                .iter()
                .any(|mapping| mapping.target == target)
            {
                return Err(crate::source_github::unavailable());
            }
        }
        self.local
            .as_ref()
            .map(|local| SourceReader {
                kind: Kind::Local(local),
                tenant,
                scope,
                target: target.to_owned(),
            })
            .ok_or_else(crate::source_github::unavailable)
    }
}
enum Kind<'a> {
    Local(&'a LocalSourceReader),
    GitHub(&'a GitHubSourceReader, &'a GitHubSourceTarget),
}
/// Exact admitted transport address, retaining no opened credentials.
pub struct SourceReader<'a> {
    kind: Kind<'a>,
    tenant: TenantId,
    scope: ScopeId,
    target: String,
}
impl SourceReader<'_> {
    /// Closed provider family for independent Configuration narrowing.
    #[must_use]
    pub fn provider(&self) -> &'static str {
        match self.kind {
            Kind::Local(_) => "local",
            Kind::GitHub(..) => "github",
        }
    }
    /// Exact read-secret identity, absent for local custody.
    pub fn secret_id(&self) -> Result<Option<TenantSecretId>> {
        match self.kind {
            Kind::Local(_) => Ok(None),
            Kind::GitHub(_, mapping) => mapping.secret_id().map(Some),
        }
    }
    /// Configured immutable provider ID, independently verified before fetch.
    #[must_use]
    pub fn repository_id(&self) -> Option<u64> {
        match self.kind {
            Kind::Local(_) => None,
            Kind::GitHub(_, mapping) => Some(mapping.repository_id),
        }
    }
    /// Preserve the qualified local digest exactly; separate remote custody domain.
    pub fn custody_digest(&self) -> Result<String> {
        let value = match self.kind {
            Kind::Local(reader) => serde_json::json!([
                "synveda.git.source.local.v1",
                self.tenant,
                self.scope,
                self.target,
                reader.repository(self.tenant, self.scope, &self.target)?
            ]),
            Kind::GitHub(_, mapping) => serde_json::json!([
                "synveda.git.source.github.v1",
                self.tenant,
                self.scope,
                self.target,
                mapping.repository_id,
                mapping.repository_uri(),
                mapping.secret_id()?
            ]),
        };
        Ok(blake3::hash(value.to_string().as_bytes())
            .to_hex()
            .to_string())
    }
    /// Resolve one branch using only the selected transport.
    pub async fn resolve(
        &self,
        credential: Option<&GitHubSourceCredential>,
        reference: &str,
    ) -> Result<SourcePin> {
        match self.kind {
            Kind::Local(reader) => {
                reader
                    .resolve(self.tenant, self.scope, &self.target, reference)
                    .await
            }
            Kind::GitHub(reader, mapping) => {
                reader
                    .resolve(
                        mapping,
                        credential.ok_or_else(crate::source_github::unavailable)?,
                        reference,
                    )
                    .await
            }
        }
    }
    /// Re-read the durable pin, preserving omission versus proven absence.
    pub async fn read_at_omitting(
        &self,
        credential: Option<&GitHubSourceCredential>,
        pin: &SourcePin,
        paths: &[String],
        omitted: &BTreeMap<String, String>,
    ) -> Result<SourceSnapshot> {
        match self.kind {
            Kind::Local(reader) => {
                reader
                    .read_at_omitting(self.tenant, self.scope, &self.target, pin, paths, omitted)
                    .await
            }
            Kind::GitHub(reader, mapping) => {
                reader
                    .read_at_omitting(
                        mapping,
                        credential.ok_or_else(crate::source_github::unavailable)?,
                        pin,
                        paths,
                        omitted,
                    )
                    .await
            }
        }
    }
}
