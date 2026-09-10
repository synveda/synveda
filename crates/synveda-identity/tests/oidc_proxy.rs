use std::time::Duration;

use synveda_identity::{OidcVerifier, parse_issuers};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn ambient_proxy_variables_cannot_escape_a_loopback_oidc_issuer() {
    let issuer_listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind issuer capture");
    let issuer_port = issuer_listener.local_addr().expect("issuer addr").port();
    let issuer = format!("http://127.0.0.1:{issuer_port}/realms/synveda");
    let proxy_listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind proxy capture");
    let proxy_port = proxy_listener.local_addr().expect("proxy addr").port();

    let issuer_task = tokio::spawn({
        let issuer = issuer.clone();
        async move {
            for _ in 0..2 {
                let (mut stream, _) = issuer_listener
                    .accept()
                    .await
                    .expect("accept issuer request");
                let mut request = vec![0u8; 4096];
                let read = stream
                    .read(&mut request)
                    .await
                    .expect("read issuer request");
                let request = String::from_utf8_lossy(&request[..read]);
                let body = if request.starts_with("GET /realms/synveda/.well-known/") {
                    serde_json::json!({
                        "issuer": issuer,
                        "authorization_endpoint": format!("{issuer}/protocol/openid-connect/auth"),
                        "token_endpoint": format!("{issuer}/protocol/openid-connect/token"),
                        "jwks_uri": format!("{issuer}/protocol/openid-connect/certs"),
                        "code_challenge_methods_supported": ["S256"],
                        "id_token_signing_alg_values_supported": ["RS256"],
                        "response_types_supported": ["code"],
                        "grant_types_supported": ["authorization_code"],
                    })
                    .to_string()
                } else if request.starts_with("GET /realms/synveda/protocol/openid-connect/certs ")
                {
                    let key: serde_json::Value =
                        serde_json::from_str(include_str!("fixtures/idp_rsa_2048.jwk.json"))
                            .expect("JWK fixture");
                    serde_json::json!({"keys": [key]}).to_string()
                } else {
                    panic!("unexpected issuer request line: {request}");
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n\
                     content-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream
                    .write_all(response.as_bytes())
                    .await
                    .expect("write issuer response");
            }
        }
    });
    let proxy_task = tokio::spawn(async move {
        match tokio::time::timeout(Duration::from_secs(1), proxy_listener.accept()).await {
            Ok(Ok((mut stream, _))) => {
                stream
                    .write_all(
                        b"HTTP/1.1 502 Bad Gateway\r\ncontent-length: 0\r\n\
                          connection: close\r\n\r\n",
                    )
                    .await
                    .expect("write proxy refusal");
                true
            }
            Ok(Err(error)) => panic!("proxy accept failed: {error}"),
            Err(_) => false,
        }
    });

    let settings = [
        "HTTP_PROXY",
        "http_proxy",
        "HTTPS_PROXY",
        "https_proxy",
        "ALL_PROXY",
        "all_proxy",
        "NO_PROXY",
        "no_proxy",
    ];
    let previous: Vec<_> = settings
        .iter()
        .map(|name| (*name, std::env::var_os(name)))
        .collect();
    let proxy_url = format!("http://127.0.0.1:{proxy_port}");
    unsafe {
        for name in [
            "HTTP_PROXY",
            "http_proxy",
            "HTTPS_PROXY",
            "https_proxy",
            "ALL_PROXY",
            "all_proxy",
        ] {
            std::env::set_var(name, &proxy_url);
        }
        std::env::set_var("NO_PROXY", "");
        std::env::set_var("no_proxy", "");
    }

    let config =
        format!(r#"[{{"issuer":"{issuer}","client_id":"synveda","audience":"synveda-api"}}]"#);
    let result = async {
        let verifier = OidcVerifier::new(parse_issuers(&config).expect("issuer config"))
            .expect("build verifier");
        tokio::time::timeout(Duration::from_secs(2), verifier.initialize())
            .await
            .map_err(|_| "OIDC initialization timed out".to_owned())?
            .map_err(|error| error.to_string())
    }
    .await;
    unsafe {
        for (name, value) in previous {
            match value {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
    }

    let proxy_observed = proxy_task.await.expect("proxy capture task");
    if proxy_observed || result.is_err() {
        issuer_task.abort();
    } else {
        issuer_task.await.expect("issuer capture task");
    }
    assert!(!proxy_observed, "ambient proxy received an OIDC request");
    result.expect("OIDC initialization stayed on loopback");
}
