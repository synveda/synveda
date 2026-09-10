//! Reserved bootstrap command.
//!
//! Deployment bootstrap belongs to the canonical Compose lifecycle. Keeping a
//! second implementation in the CLI previously retained an obsolete bundled
//! identity provider and host-process gateway behavior, so this verb fails
//! before it reads configuration or mutates state.

pub async fn init() -> Result<(), String> {
    Err(
        "synveda init is not a deployment lifecycle. Use the canonical Docker Compose reference documented in docs/INSTALL.md; bootstrap remains deployment-owned and this command reads no configuration and changes no state"
            .to_owned(),
    )
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn init_is_a_side_effect_free_refusal() {
        let refusal = super::init()
            .await
            .expect_err("the reserved init verb must fail closed");
        assert!(refusal.contains("not a deployment lifecycle"), "{refusal}");
        assert!(refusal.contains("docs/INSTALL.md"), "{refusal}");
        assert!(refusal.contains("reads no configuration"), "{refusal}");
        assert!(refusal.contains("changes no state"), "{refusal}");
    }
}
