use synveda_identity::{OidcVerifier, parse_issuers};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn private_discovery_preserves_the_public_issuer_and_endpoint_origins() {
    for mutation in ["valid", "issuer", "authorization", "token", "jwks"] {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("fixture listener");
        let backend = format!("http://{}", listener.local_addr().expect("fixture address"));
        let public = "http://localhost:18080/realms/synveda";
        let mut document = serde_json::json!({
            "issuer": public,
            "authorization_endpoint": format!("{public}/auth"),
            "token_endpoint": format!("{backend}/token"),
            "jwks_uri": format!("{backend}/certs"),
            "code_challenge_methods_supported": ["S256"],
            "id_token_signing_alg_values_supported": ["RS256"],
            "response_types_supported": ["code"],
            "grant_types_supported": ["authorization_code"],
        });
        let field = match mutation {
            "issuer" => Some("issuer"),
            "authorization" => Some("authorization_endpoint"),
            "token" => Some("token_endpoint"),
            "jwks" => Some("jwks_uri"),
            _ => None,
        };
        if let Some(field) = field {
            document[field] = serde_json::json!("http://untrusted.invalid/endpoint");
        }
        let task = tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.expect("accept fixture");
                let mut request = [0; 4096];
                let count = stream.read(&mut request).await.expect("read fixture");
                let body = if request[..count].starts_with(b"GET /discovery ") {
                    document.to_string()
                } else {
                    let key: serde_json::Value =
                        serde_json::from_str(include_str!("fixtures/idp_rsa_2048.jwk.json"))
                            .expect("fixture key");
                    serde_json::json!({"keys": [key]}).to_string()
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).await.expect("reply");
            }
        });
        let config = serde_json::json!([{
            "issuer": public, "discovery_url": format!("{backend}/discovery"),
            "client_id": "synveda", "audience": "synveda-api"
        }]);
        let verifier = OidcVerifier::new_with_transport(
            parse_issuers(&config.to_string()).expect("config"),
            true,
            None,
        )
        .expect("verifier");
        let result = verifier.initialize().await;
        task.abort();
        assert_eq!(
            result.is_ok(),
            mutation == "valid",
            "{mutation}: {result:?}"
        );
    }
}

#[test]
fn configured_backchannel_cannot_relax_the_issuer_transport_policy() {
    for (issuer, development, accepted) in [
        ("https://auth.example.com/realms/synveda", false, false),
        ("https://auth.example.com/realms/synveda", true, false),
        ("http://localhost:8080/realms/synveda", false, false),
        ("http://localhost:8080/realms/synveda", true, true),
    ] {
        let config = serde_json::json!([{
            "issuer": issuer,
            "discovery_url": "http://keycloak:8080/realms/synveda/.well-known/openid-configuration",
            "client_id": "synveda", "audience": "synveda-api"
        }]);
        assert_eq!(
            OidcVerifier::new_with_transport(
                parse_issuers(&config.to_string()).expect("config"),
                development,
                None,
            )
            .is_ok(),
            accepted
        );
    }
}
