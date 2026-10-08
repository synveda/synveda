//! Deployment selection narrows a governed destination ID. A remote mapping
//! never falls back to local custody if its credential or provider is absent.

use synveda_types::{Error, Result, ScopeId, TenantId, TenantSecretId};

use crate::{
    GitHubCredential, GitHubTarget, GitHubTransport, LocalTransport, Projection, Transport,
    validate_target,
};

/// Optional local transport and private GitHub allowlist used by the gateway.
#[derive(Clone)]
pub struct ExportTransport {
    local: Option<LocalTransport>,
    github: Option<GitHubTransport>,
}

impl ExportTransport {
    /// Assemble explicitly configured transports; no outbound target is enabled
    /// by this constructor without independent governed Configuration.
    #[must_use]
    pub fn new(local: Option<LocalTransport>, github: Option<GitHubTransport>) -> Self {
        Self { local, github }
    }

    /// Select the exact deployment destination for a tenant/scope/target.
    pub fn destination(
        &self,
        tenant: TenantId,
        scope: ScopeId,
        target: &str,
    ) -> Result<Destination<'_>> {
        validate_target(target)?;
        if let Some(github) = &self.github {
            if let Some(mapping) = github
                .targets
                .iter()
                .find(|m| m.tenant_id == tenant && m.scope_id == scope && m.target == target)
            {
                return Ok(Destination {
                    kind: Kind::GitHub(github, mapping),
                });
            }
            if github.targets.iter().any(|m| m.target == target) {
                return Err(crate::github::unavailable());
            }
        }
        self.local
            .as_ref()
            .map(|local| Destination {
                kind: Kind::Local(local),
            })
            .ok_or_else(|| Error::Invalid {
                message: "Git export destination is disabled by deployment configuration".into(),
            })
    }
}

/// A validated destination. Its identity digest is retained with disclosure
/// intent so changing deployment custody cannot retarget a prepared receipt.
#[derive(Clone, Copy)]
pub struct Destination<'a> {
    kind: Kind<'a>,
}

#[derive(Clone, Copy)]
enum Kind<'a> {
    Local(&'a LocalTransport),
    GitHub(&'a GitHubTransport, &'a GitHubTarget),
}

impl Destination<'_> {
    /// Closed provider label for Configuration narrowing, audit and responses.
    #[must_use]
    pub const fn provider(&self) -> &'static str {
        match self.kind {
            Kind::Local(_) => "local",
            Kind::GitHub(_, _) => "github",
        }
    }

    /// Stable credential identity, absent for local export.
    pub fn secret_id(&self) -> Result<Option<TenantSecretId>> {
        match self.kind {
            Kind::Local(_) => Ok(None),
            Kind::GitHub(_, target) => target.secret_id().map(Some),
        }
    }

    /// Immutable repository identity, absent for local export.
    #[must_use]
    pub const fn repository_id(&self) -> Option<u64> {
        match self.kind {
            Kind::Local(_) => None,
            Kind::GitHub(_, target) => Some(target.repository_id),
        }
    }

    /// Content-free digest of the exact destination and credential custody.
    /// Local absolute paths contribute to the digest but are never returned.
    pub fn digest(&self, tenant: TenantId, scope: ScopeId, target: &str) -> Result<String> {
        let value = match self.kind {
            Kind::Local(local) => {
                serde_json::json!(["local", local.repository(tenant, scope, target)?])
            }
            Kind::GitHub(_, mapping) => serde_json::json!([
                "github",
                tenant,
                scope,
                target,
                mapping.owner,
                mapping.repository,
                mapping.repository_id,
                mapping.secret_id()?
            ]),
        };
        Ok(
            blake3::hash(&serde_json::to_vec(&value).map_err(|_| crate::github::failure())?)
                .to_hex()
                .to_string(),
        )
    }

    /// Advance the exact prepared result, or verify a completed replay. The
    /// current opened credential is supplied by the authorized application.
    pub async fn advance(
        &self,
        projection: &Projection,
        previous: Option<&str>,
        completed: bool,
        credential: Option<&GitHubCredential>,
    ) -> Result<bool> {
        match self.kind {
            Kind::Local(local) => {
                if credential.is_some() {
                    return Err(crate::github::unavailable());
                }
                if completed
                    && local.observed(projection).await?.as_deref() != Some(&projection.head)
                {
                    return Err(Error::Conflict {
                        message: "Git export destination diverged from its completed cursor".into(),
                    });
                }
                local.advance(projection, previous).await
            }
            Kind::GitHub(github, target) => {
                github
                    .advance(
                        target,
                        credential.ok_or_else(crate::github::unavailable)?,
                        projection,
                        previous,
                        completed,
                    )
                    .await
            }
        }
    }
}
