//! Authenticated context preview and delivery inspection (CTX-8).
//!
//! Both commands use the public gateway. The preview route runs the same
//! policy-filtered planner without writing a ContextRun delivery; the CLI
//! never turns a preview into an activation or provider-usage assertion.

use serde_json::{Value, json};
use synveda_types::{ContextRunId, KnowledgeItemId, KnowledgeRevisionId, Sensitivity, SessionId};

use crate::api::{Api, Origin};

pub struct PreviewOptions<'a> {
    pub session: SessionId,
    pub query: Option<&'a str>,
    pub budget_tokens: Option<u32>,
    pub tokenizer_encoding: Option<&'a str>,
    pub required_knowledge_revisions: &'a [String],
    pub max_sensitivity: Option<Sensitivity>,
    pub json: bool,
}

async fn connect(profile: &str) -> Result<Api, String> {
    let (api, origin) = Api::connect(profile).await?;
    match origin {
        Origin::Profile(name) => eprintln!("context as {} (profile {name})", api.subject),
        Origin::Environment => eprintln!("context as {} (SYNVEDA_TOKEN)", api.subject),
    }
    Ok(api)
}

pub async fn preview(profile: &str, options: PreviewOptions<'_>) -> Result<(), String> {
    if options.budget_tokens == Some(0) {
        return Err("--budget-tokens must be at least 1".to_owned());
    }
    let required = options
        .required_knowledge_revisions
        .iter()
        .map(|address| {
            let (item, revision) = address
                .split_once('@')
                .ok_or_else(|| "--require-knowledge expects ITEM_ID@REVISION_ID".to_owned())?;
            let item_id: KnowledgeItemId = item
                .parse()
                .map_err(|_| "--require-knowledge has an invalid item id".to_owned())?;
            let revision_id: KnowledgeRevisionId = revision
                .parse()
                .map_err(|_| "--require-knowledge has an invalid revision id".to_owned())?;
            Ok(json!({"item_id": item_id, "revision_id": revision_id}))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let api = connect(profile).await?;
    let response = api
        .post(
            &format!("/v1/sessions/{}/context-preview", options.session),
            Some(json!({
                "query": options.query,
                "budget_tokens": options.budget_tokens,
                "tokenizer_encoding": options.tokenizer_encoding,
                "required_knowledge_revisions": required,
                "max_sensitivity": options.max_sensitivity,
            })),
        )
        .await?;
    if options.json {
        return print_json(&response);
    }
    println!("Preview only — no ContextRun delivery or provider request recorded.");
    print_accounting(&response);
    print_sources(&response["selected"], "Selected");
    print_sources(&response["omitted"], "Omitted");
    if let Some(message) = response["policy_exclusion_message"].as_str() {
        println!("{message}");
    }
    println!(
        "\nRendered Synveda context:\n{}",
        response["rendered"].as_str().unwrap_or("")
    );
    Ok(())
}

pub async fn inspect(profile: &str, run: ContextRunId, json: bool) -> Result<(), String> {
    let api = connect(profile).await?;
    let response = api.get(&format!("/v1/context-runs/{run}")).await?;
    if json {
        return print_json(&response);
    }
    let recorded = &response["run"];
    println!("Recorded ContextRun {run}");
    print_accounting(recorded);
    println!("Observed provider usage: unavailable through this route");
    let selections = response["selections"].as_array();
    println!(
        "Selected revisions visible now: {}",
        selections.map_or(0, Vec::len)
    );
    if let Some(selections) = selections {
        for selection in selections {
            let address = selection["knowledge_revision_id"]
                .as_str()
                .or_else(|| selection["capture_candidate_id"].as_str())
                .unwrap_or("address withheld");
            let status = if selection["reason_codes"]
                .as_array()
                .is_some_and(|reasons| reasons.iter().any(|reason| reason == "excerpt"))
            {
                "excerpt"
            } else {
                "complete or status unavailable"
            };
            println!("  {address}: {status}");
        }
    }
    if let Some(candidates) = response["candidates"].as_array() {
        for candidate in candidates {
            if let Some(reason) = candidate["exclusion_reason"].as_str() {
                let address = candidate["knowledge_revision_id"]
                    .as_str()
                    .or_else(|| candidate["capture_candidate_id"].as_str())
                    .unwrap_or("address withheld");
                println!("  omitted {address}: {reason}");
            }
        }
    }
    if let Some(message) = response["policy_exclusion_message"].as_str() {
        println!("{message}");
    }
    Ok(())
}

pub async fn detail(
    profile: &str,
    item: KnowledgeItemId,
    revision: KnowledgeRevisionId,
    json: bool,
) -> Result<(), String> {
    let api = connect(profile).await?;
    let mut cursor: Option<String> = None;
    for _ in 0..10 {
        let mut path = format!("/v1/knowledge/{item}/history?limit=200");
        if let Some(value) = &cursor {
            let encoded = url::form_urlencoded::Serializer::new(String::new())
                .append_pair("cursor", value)
                .finish();
            path.push('&');
            path.push_str(&encoded);
        }
        let response = api.get(&path).await?;
        if let Some(found) = response["revisions"].as_array().and_then(|revisions| {
            revisions
                .iter()
                .find(|entry| entry["id"] == revision.to_string())
        }) {
            if json {
                return print_json(found);
            }
            println!(
                "Knowledge {item}@{revision} — {}",
                found["title"].as_str().unwrap_or("")
            );
            println!("{}", found["body_markdown"].as_str().unwrap_or(""));
            return Ok(());
        }
        cursor = response["next_cursor"].as_str().map(str::to_owned);
        if cursor.is_none() {
            return Err("Knowledge revision unavailable or not visible".to_owned());
        }
    }
    Err("Knowledge history exceeded the 2,000-revision CLI scan limit".to_owned())
}

fn print_accounting(value: &Value) {
    let mode = value["optimization_mode"].as_str().unwrap_or("unknown");
    let certainty = value["token_count_kind"].as_str().unwrap_or("estimated");
    let encoding = value["tokenizer_encoding"]
        .as_str()
        .unwrap_or("unknown model/encoding");
    println!("Mode: {mode}");
    println!(
        "Synveda rendered text: {} {certainty} tokens / {} budget ({encoding})",
        value["tokens"], value["budget_tokens"]
    );
    println!(
        "Boundary: Synveda-rendered text only; host history, other tools and output reservation are outside this count."
    );
}

fn print_sources(value: &Value, label: &str) {
    if let Some(entries) = value.as_array() {
        println!("{label}: {} policy-visible source(s)", entries.len());
        for entry in entries {
            let address = entry["knowledge_revision_id"]
                .as_str()
                .or_else(|| entry["capture_candidate_id"].as_str())
                .unwrap_or("address withheld");
            let status = entry["omission_reason"].as_str().unwrap_or_else(|| {
                if entry["reason_codes"]
                    .as_array()
                    .is_some_and(|reasons| reasons.iter().any(|reason| reason == "excerpt"))
                {
                    "exact excerpt"
                } else {
                    "selected complete"
                }
            });
            let required = if entry["required"] == true {
                " (required)"
            } else {
                ""
            };
            println!("  {address}: {status}{required}");
        }
    }
}

fn print_json(value: &Value) -> Result<(), String> {
    println!(
        "{}",
        serde_json::to_string_pretty(value).map_err(|error| error.to_string())?
    );
    Ok(())
}
