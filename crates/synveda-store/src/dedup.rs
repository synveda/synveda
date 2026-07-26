//! The dedup nominator's storage side (MEM-5, ADR-0039): the MinHash/LSH
//! encoding of a record's content, the two nomination queries, and the
//! supersession edge.
//!
//! The encoding lives here rather than in `synveda-ingest` because it *is*
//! the `record_signatures` columns — the same argument that keeps the audit
//! chain's canonical form in `synveda-audit` and the object address in
//! `synveda-vedaflow`. Two writers that disagree about how a signature is
//! computed do not collide, and an LSH index whose entries do not collide is
//! an index that finds nothing.
//!
//! What is deliberately *not* here: the decision. Whether a nominated pair is
//! a restatement, a contradiction, or two true facts is the judge's, and the
//! judge is policy — `synveda-ingest` owns it, configured per pack (ADR-0039
//! decisions 5 and 12; seed §2.4's "storage knows nothing of policy").

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use synveda_types::{Error, RecordClass, RecordId, Result, ScopeId, TenantId};
use uuid::Uuid;

use crate::records::{RecordRow, RecordVersion, storage_error};

/// MinHash permutations per signature. 96 keeps the standard-error of the
/// Jaccard estimate near `1/sqrt(96) ≈ 0.10` — plenty for *nomination*,
/// which is all the signature is used for: every pair the bands surface is
/// scored exactly afterwards, from content.
pub const MINHASH_PERMUTATIONS: usize = 96;

/// LSH bands, and rows per band. `32 × 3 = 96`, putting the collision
/// threshold at `(1/32)^(1/3) ≈ 0.32` Jaccard *over frames* — where the
/// pairs worth judging sit at 0.6 and above, so their collision probability
/// is above 0.999 while an unrelated pair's stays a few per cent.
///
/// Tuned for recall on purpose: a nomination the judge refuses costs a row,
/// and a nomination that never happens costs a fact (ADR-0039's context
/// section on the asymmetry).
pub const LSH_BANDS: usize = 32;
/// Rows per band; `LSH_BANDS * LSH_ROWS == MINHASH_PERMUTATIONS`.
pub const LSH_ROWS: usize = 3;

/// A record's MinHash signature and the bands it collides on — exactly the
/// `record_signatures` row, minus the keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordSignature {
    /// The [`MINHASH_PERMUTATIONS`] minima.
    pub signature: Vec<i64>,
    /// The [`LSH_BANDS`] band hashes — the nomination predicate's terms.
    pub bands: Vec<i64>,
}

/// One nominated neighbour: a current record version at the candidate's own
/// scope, and its cosine distance to the candidate when the semantic leg is
/// what surfaced it.
///
/// `distance` is `None` for a purely lexical nomination, and that is not a
/// gap to fill: the two legs answer different questions, and a neighbour the
/// bands found is scored on Jaccard, which is exact rather than estimated
/// (ADR-0039 decision 2).
#[derive(Debug, Clone)]
pub struct Neighbour {
    /// The neighbour's current version.
    pub version: RecordVersion,
    /// Cosine distance to the candidate vector (smaller is nearer), when the
    /// dense leg produced this neighbour under a comparable model.
    pub distance: Option<f64>,
}

/// Function words carried by almost every sentence. They belong in the
/// Jaccard score — two statements that share their grammar *are* more alike —
/// and not in the frame, which is trying to answer "are these about the same
/// thing".
///
/// Small and English on purpose. It is a heuristic guard, one of the judge's
/// three conjuncts (ADR-0039 decision 5), and every word missing from it can
/// only make the judge *refuse* more often.
const STOPWORDS: [&str; 62] = [
    "a", "about", "after", "all", "also", "am", "an", "and", "any", "are", "as", "at", "be",
    "been", "but", "by", "can", "did", "do", "does", "for", "from", "had", "has", "have", "he",
    "her", "his", "how", "i", "if", "in", "into", "is", "it", "its", "me", "my", "no", "not",
    "now", "of", "on", "one", "or", "our", "out", "she", "so", "than", "that", "the", "their",
    "them", "then", "there", "they", "this", "to", "was", "we", "with",
];

/// Tokens that name a *value* rather than a subject: anything with a digit,
/// plus the weekday and month names. Held out of the frame and compared on
/// their own, because "same subject, changed number" is the shape of most
/// knowledge updates — and a frame that included the number would score an
/// update as *less* similar the more clearly it was one (ADR-0039
/// decision 5).
const VALUE_WORDS: [&str; 26] = [
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
    "sunday",
    "mondays",
    "tuesdays",
    "wednesdays",
    "thursdays",
    "fridays",
    "saturdays",
    "sundays",
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];

/// Three views of one content string, built in one pass (ADR-0039
/// decisions 3 and 5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentTokens {
    /// Every normalised token, as a sorted set — what exact Jaccard scores.
    pub all: Vec<String>,
    /// The frame: content words in document order, deduplicated, with
    /// stopwords and value tokens removed. What the signature hashes and
    /// what the judge asks its overlap question about. Document order is
    /// kept because the judge's subject proxy is the leading token.
    pub frame: Vec<String>,
    /// The value tokens, as a sorted set — the "did something change" half
    /// of the judge's rule.
    pub values: Vec<String>,
}

/// Normalises `content` into its three views: lowercased, whitespace-split,
/// outer punctuation trimmed.
///
/// Word unigrams rather than character shingles, on ADR-0039 option 5's
/// numbers: a knowledge update re-words around a changed value, and the
/// canonical case ("we deploy on Tuesdays" against "…Thursdays") is J ≈ 0.46
/// in 5-grams — sitting on the band threshold — against J = 0.60 in unigrams.
/// Order-blindness costs the *signature* nothing, because it only nominates.
///
/// Punctuation is trimmed from the ends and kept inside, so `docs/deploy.md`,
/// `10:15` and `[REDACTED:secret]` stay single tokens.
#[must_use]
pub fn tokenise(content: &str) -> ContentTokens {
    let mut all: BTreeSet<String> = BTreeSet::new();
    let mut values: BTreeSet<String> = BTreeSet::new();
    let mut frame: Vec<String> = Vec::new();
    for word in content.split_whitespace() {
        let token = word
            .trim_matches(|c: char| !c.is_alphanumeric() && c != '[' && c != ']')
            .to_lowercase();
        if token.is_empty() {
            continue;
        }
        let is_value =
            token.chars().any(|c| c.is_ascii_digit()) || VALUE_WORDS.contains(&token.as_str());
        if is_value {
            values.insert(token.clone());
        } else if !STOPWORDS.contains(&token.as_str()) && !frame.contains(&token) {
            frame.push(token.clone());
        }
        all.insert(token);
    }
    if all.is_empty() {
        // Content with no word characters at all still needs a signature the
        // CHECK constraints accept, and still deserves to collide with an
        // identical restatement of itself.
        all.insert(content.trim().to_lowercase());
    }
    ContentTokens {
        all: all.into_iter().collect(),
        frame,
        values: values.into_iter().collect(),
    }
}

/// The normalised token set of `content` — [`tokenise`]'s `all` view, for
/// callers that want nothing else.
#[must_use]
pub fn normalised_tokens(content: &str) -> Vec<String> {
    tokenise(content).all
}

/// Exact Jaccard over two normalised token sets, both assumed sorted and
/// deduplicated (what [`normalised_tokens`] returns). Two empty sets are
/// identical, which is the only sensible reading of `0/0` here.
#[must_use]
pub fn jaccard(a: &[String], b: &[String]) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let intersection = a.iter().filter(|token| b.contains(token)).count();
    let union = a.len() + b.len() - intersection;
    if union == 0 {
        return 1.0;
    }
    intersection as f64 / union as f64
}

/// The MinHash signature and LSH bands of `content`.
///
/// Deterministic across processes and releases: the token hash is BLAKE3 and
/// the permutations are fixed-seed SplitMix64, so a record signed by one
/// worker collides with a candidate hashed by another. Changing any of it
/// changes what collides, which is a re-signing migration, not a refactor.
#[must_use]
pub fn signature(content: &str) -> RecordSignature {
    signature_of(&tokenise(content))
}

/// [`signature`] over already-normalised tokens — the caller that has just
/// built them for scoring should not build them twice.
///
/// **The frame is what is hashed**, not the full token set, and that is the
/// decision that makes the lexical leg work: a knowledge update keeps its
/// subject and changes a value, so its frames sit at 0.6–1.0 Jaccard where
/// its full token sets sit at 0.4 — under the band threshold, where the
/// pairs the AC depends on would be nominated by coin flip. Scoring still
/// uses the full set: two statements whose frames are identical because only
/// a number changed are precisely *not* duplicates.
///
/// Content that is nothing but stopwords and numbers has an empty frame and
/// falls back to the full set, so it still collides with an identical
/// restatement of itself and the row still satisfies its CHECK constraints.
#[must_use]
pub fn signature_of(tokens: &ContentTokens) -> RecordSignature {
    let hashed = if tokens.frame.is_empty() {
        &tokens.all
    } else {
        &tokens.frame
    };
    let hashes: Vec<u64> = hashed.iter().map(|token| token_hash(token)).collect();
    let mut signature = Vec::with_capacity(MINHASH_PERMUTATIONS);
    for permutation in 0..MINHASH_PERMUTATIONS {
        let seed = permutation as u64;
        let minimum = hashes
            .iter()
            .map(|hash| splitmix64(hash ^ splitmix64(seed)))
            .min()
            // `tokenise` never leaves both views empty, so this is
            // unreachable through [`signature`]; a caller passing empty
            // tokens gets a well-defined constant rather than a panic.
            .unwrap_or(u64::MAX);
        signature.push(minimum as i64);
    }
    let bands = (0..LSH_BANDS)
        .map(|band| {
            let mut hasher = blake3::Hasher::new();
            // The band index is in the hash: the same four minima in two
            // different bands must not be the same bucket.
            hasher.update(&(band as u64).to_le_bytes());
            for row in 0..LSH_ROWS {
                hasher.update(&signature[band * LSH_ROWS + row].to_le_bytes());
            }
            let digest = hasher.finalize();
            i64::from_le_bytes(digest.as_bytes()[..8].try_into().expect("8 bytes"))
        })
        .collect();
    RecordSignature { signature, bands }
}

/// BLAKE3 of a token, taken as a `u64`.
fn token_hash(token: &str) -> u64 {
    let digest = blake3::hash(token.as_bytes());
    u64::from_le_bytes(digest.as_bytes()[..8].try_into().expect("8 bytes"))
}

/// SplitMix64 — the permutation family. Standard, deterministic, and
/// dependency-free.
fn splitmix64(input: u64) -> u64 {
    let mut z = input.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// The comparison group: what a candidate is allowed to be compared with
/// (ADR-0039 decision 9).
///
/// Same scope, same owner, same class, derived-kind only, and still valid at
/// or after the candidate's own instant. Each field is a refusal: a session
/// may not reach another scope's material, may not close a fact somebody else
/// asserted, may not let a `procedure` collide with an `episode`, and may not
/// touch authored material at all.
#[derive(Debug, Clone, Copy)]
pub struct CandidateGroup {
    /// The tenant.
    pub tenant_id: TenantId,
    /// The owner's home scope — where the candidate will land.
    pub scope_id: ScopeId,
    /// The owning identity.
    pub owner_id: synveda_types::IdentityId,
    /// What the candidate asserts.
    pub class: RecordClass,
    /// The candidate's valid-from: neighbours whose window already closed
    /// before this instant are history and are not nominated.
    pub at: DateTime<Utc>,
}

/// The lexical leg: current records in the group whose LSH bands overlap
/// `bands` (ADR-0039 decision 2).
///
/// Ordered by **how many bands they share**, which is a monotone proxy for
/// frame similarity — so when the cap binds it keeps the most similar
/// neighbours rather than the newest. That is what lets the band threshold
/// sit low enough for recall without the cap quietly becoming the filter.
/// Ties break on newest-first, then id, so the result is a total order.
#[tracing::instrument(
    name = "store.dedup.nominate_lexical",
    skip_all,
    fields(tenant.id = %group.tenant_id, scope.id = %group.scope_id, limit),
    err(Display)
)]
pub async fn nominate_lexical(
    conn: &mut PgConnection,
    group: &CandidateGroup,
    bands: &[i64],
    limit: i64,
) -> Result<Vec<RecordVersion>> {
    let rows = sqlx::query_as!(
        RecordRow,
        r#"
        select r.id, r.tenant_id, r.scope_id, r.owner_id, r.kind, r.class,
               r.content, r.sensitivity, r.provenance, r.valid_from, r.valid_to,
               r.tx_from, r.tx_to
        from record_signatures s
        join records r on r.id = s.record_id
        where s.tenant_id = $1
          and s.bands && $2::bigint[]
          and r.tenant_id = $1
          and r.scope_id = $3
          and r.owner_id = $4
          and r.class = $5
          and r.kind = 'derived'
          and (r.valid_to is null or r.valid_to > $6)
        order by (select count(*) from unnest(s.bands) as band
                  where band = any($2::bigint[])) desc,
                 r.valid_from desc, r.id
        limit $7
        "#,
        group.tenant_id.as_uuid(),
        bands,
        group.scope_id.as_uuid(),
        group.owner_id.as_uuid(),
        group.class.as_str(),
        group.at,
        limit,
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(TryInto::try_into).collect()
}

/// The semantic leg: the nearest current records in the group to
/// `query_vector`, among vectors the same `model` wrote. Dispatches on the
/// vector's dimension exactly as [`crate::search::dense_candidates`] does,
/// and refuses an unsupported one by naming the supported set.
///
/// Must run inside a tenant transaction: the HNSW GUCs are transaction-local,
/// and without iterative scan a group predicate this selective starves the
/// limit (ADR-0024 decision 5).
#[tracing::instrument(
    name = "store.dedup.nominate_dense",
    skip_all,
    fields(tenant.id = %group.tenant_id, scope.id = %group.scope_id, dim = query_vector.len(), limit),
    err(Display)
)]
pub async fn nominate_dense(
    conn: &mut PgConnection,
    group: &CandidateGroup,
    model: &str,
    query_vector: &[f32],
    limit: i64,
) -> Result<Vec<(RecordVersion, f64)>> {
    sqlx::query!(
        r#"
        select set_config('hnsw.iterative_scan', 'relaxed_order', true) as "a!",
               set_config('hnsw.ef_search', '100', true) as "b!"
        "#
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(storage_error)?;
    let rows = match query_vector.len() {
        16 => {
            sqlx::query_as!(
                NeighbourRow,
                r#"
                select r.id as "id!", r.tenant_id as "tenant_id!",
                       r.scope_id as "scope_id!", r.owner_id as "owner_id!",
                       r.kind as "kind!", r.class as "class!", r.content as "content!",
                       r.sensitivity as "sensitivity!", r.provenance as "provenance!",
                       r.valid_from as "valid_from!", r.valid_to,
                       r.tx_from as "tx_from!", r.tx_to,
                       (e.embedding::vector(16) <=> $7::real[]::vector(16))::float8
                           as "distance!"
                from record_embeddings e
                join records r on r.id = e.record_id
                where e.tenant_id = $1
                  and e.dim = 16
                  and e.model = $2
                  and r.tenant_id = $1
                  and r.scope_id = $3
                  and r.owner_id = $4
                  and r.class = $5
                  and r.kind = 'derived'
                  and (r.valid_to is null or r.valid_to > $6)
                order by e.embedding::vector(16) <=> $7::real[]::vector(16)
                limit $8
                "#,
                group.tenant_id.as_uuid(),
                model,
                group.scope_id.as_uuid(),
                group.owner_id.as_uuid(),
                group.class.as_str(),
                group.at,
                query_vector,
                limit,
            )
            .fetch_all(&mut *conn)
            .await
        }
        1024 => {
            sqlx::query_as!(
                NeighbourRow,
                r#"
                select r.id as "id!", r.tenant_id as "tenant_id!",
                       r.scope_id as "scope_id!", r.owner_id as "owner_id!",
                       r.kind as "kind!", r.class as "class!", r.content as "content!",
                       r.sensitivity as "sensitivity!", r.provenance as "provenance!",
                       r.valid_from as "valid_from!", r.valid_to,
                       r.tx_from as "tx_from!", r.tx_to,
                       (e.embedding::vector(1024) <=> $7::real[]::vector(1024))::float8
                           as "distance!"
                from record_embeddings e
                join records r on r.id = e.record_id
                where e.tenant_id = $1
                  and e.dim = 1024
                  and e.model = $2
                  and r.tenant_id = $1
                  and r.scope_id = $3
                  and r.owner_id = $4
                  and r.class = $5
                  and r.kind = 'derived'
                  and (r.valid_to is null or r.valid_to > $6)
                order by e.embedding::vector(1024) <=> $7::real[]::vector(1024)
                limit $8
                "#,
                group.tenant_id.as_uuid(),
                model,
                group.scope_id.as_uuid(),
                group.owner_id.as_uuid(),
                group.class.as_str(),
                group.at,
                query_vector,
                limit,
            )
            .fetch_all(&mut *conn)
            .await
        }
        unsupported => {
            return Err(Error::Invalid {
                message: format!(
                    "no ANN index for {unsupported}-dimension vectors; supported: \
                     {:?} (ADR-0024 decision 5)",
                    crate::search::SUPPORTED_ANN_DIMS
                ),
            });
        }
    };
    rows.map_err(storage_error)?
        .into_iter()
        .map(|row| {
            let distance = row.distance;
            RecordVersion::try_from(RecordRow {
                id: row.id,
                tenant_id: row.tenant_id,
                scope_id: row.scope_id,
                owner_id: row.owner_id,
                kind: row.kind,
                class: row.class,
                content: row.content,
                sensitivity: row.sensitivity,
                provenance: row.provenance,
                valid_from: row.valid_from,
                valid_to: row.valid_to,
                tx_from: row.tx_from,
                tx_to: row.tx_to,
            })
            .map(|version| (version, distance))
        })
        .collect()
}

struct NeighbourRow {
    id: Uuid,
    tenant_id: Uuid,
    scope_id: Uuid,
    owner_id: Uuid,
    kind: String,
    class: String,
    content: String,
    sensitivity: String,
    provenance: serde_json::Value,
    valid_from: DateTime<Utc>,
    valid_to: Option<DateTime<Utc>>,
    tx_from: DateTime<Utc>,
    tx_to: Option<DateTime<Utc>>,
    distance: f64,
}

/// One supersession, as the pipeline decided it.
#[derive(Debug, Clone)]
pub struct Supersession {
    /// The record whose window closed.
    pub superseded_id: RecordId,
    /// The record that closed it.
    pub superseding_id: RecordId,
    /// The judge that decided (`deterministic` today).
    pub method: String,
    /// The verdict class, machine-readable and short.
    pub reason: String,
    /// Jaccard as integer per mille, when the lexical leg scored the pair.
    pub jaccard_permille: Option<i32>,
    /// Cosine as integer per mille, when the semantic leg did.
    pub cosine_permille: Option<i32>,
    /// The instant the superseded record's window was closed at.
    pub closed_at: DateTime<Utc>,
}

/// Writes the supersession edge (ADR-0039 decision 7). Idempotent: a pair
/// already recorded is left as it stands, so a redelivered group cannot
/// rewrite the reason a window closed for.
#[tracing::instrument(
    name = "store.dedup.record_supersession",
    skip_all,
    fields(
        tenant.id = %tenant_id,
        record.superseded = %edge.superseded_id,
        record.superseding = %edge.superseding_id,
    ),
    err(Display)
)]
pub async fn record_supersession(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    edge: &Supersession,
) -> Result<bool> {
    let result = sqlx::query!(
        r#"
        insert into record_supersessions
            (tenant_id, superseded_id, superseding_id, method, reason,
             jaccard_permille, cosine_permille, closed_at)
        values ($1, $2, $3, $4, $5, $6, $7, $8)
        on conflict (superseded_id, superseding_id) do nothing
        "#,
        tenant_id.as_uuid(),
        edge.superseded_id.as_uuid(),
        edge.superseding_id.as_uuid(),
        edge.method,
        edge.reason,
        edge.jaccard_permille,
        edge.cosine_permille,
        edge.closed_at,
    )
    .execute(&mut *conn)
    .await
    .map_err(storage_error)?;
    Ok(result.rows_affected() > 0)
}

/// Every edge naming `record_id` on either side, newest decision first —
/// "what closed this, and what did it close". The audit trail's join, and
/// what a review surface would read (AUD-2, CNSL-2).
#[tracing::instrument(
    name = "store.dedup.supersessions_for",
    skip_all,
    fields(tenant.id = %tenant_id, record.id = %record_id),
    err(Display)
)]
pub async fn supersessions_for(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    record_id: RecordId,
) -> Result<Vec<Supersession>> {
    let rows = sqlx::query!(
        r#"
        select superseded_id, superseding_id, method, reason,
               jaccard_permille, cosine_permille, closed_at
        from record_supersessions
        where tenant_id = $1 and (superseded_id = $2 or superseding_id = $2)
        order by decided_at desc, superseded_id
        "#,
        tenant_id.as_uuid(),
        record_id.as_uuid(),
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(storage_error)?;
    Ok(rows
        .into_iter()
        .map(|row| Supersession {
            superseded_id: RecordId::from_uuid(row.superseded_id),
            superseding_id: RecordId::from_uuid(row.superseding_id),
            method: row.method,
            reason: row.reason,
            jaccard_permille: row.jaccard_permille,
            cosine_permille: row.cosine_permille,
            closed_at: row.closed_at,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(text: &str) -> Vec<String> {
        normalised_tokens(text)
    }

    #[test]
    fn tokens_are_lowercased_deduplicated_and_outer_punctuation_free() {
        assert_eq!(
            tokens("The runbook, THE runbook; lives in docs/deploy.md."),
            vec![
                "docs/deploy.md".to_owned(),
                "in".to_owned(),
                "lives".to_owned(),
                "runbook".to_owned(),
                "the".to_owned(),
            ]
        );
        // Placeholders stay whole: MEM-2's `[REDACTED:*]` is one opaque
        // token, and splitting it would make two records that hid different
        // secrets look alike.
        assert!(tokens("token [REDACTED:secret] here").contains(&"[redacted:secret]".to_owned()));
        // Content with no word characters still yields a token, so the
        // CHECK constraints on the signature columns can never be breached.
        assert_eq!(tokens("!!!"), vec!["!!!".to_owned()]);
        assert!(!tokens("   ").is_empty());
    }

    /// The frame keeps the subject in document order and drops what every
    /// sentence carries; values leave the frame and are kept for the judge's
    /// "did something change" question.
    #[test]
    fn the_frame_is_the_subject_and_the_values_are_kept_apart() {
        let parsed = tokenise("The stand-up is at 09:30 on Tuesdays every weekday");
        assert_eq!(
            parsed.frame,
            vec![
                "stand-up".to_owned(),
                "every".to_owned(),
                "weekday".to_owned()
            ],
            "document order, no stopwords, no values"
        );
        assert_eq!(
            parsed.values,
            vec!["09:30".to_owned(), "tuesdays".to_owned()],
            "a digit or a weekday name makes a value"
        );
        assert!(
            parsed.all.contains(&"the".to_owned()),
            "scoring sees it all"
        );

        // All stopwords and numbers: an empty frame, and the signature falls
        // back to the full set rather than producing an unstorable row.
        let bare = tokenise("it is at 10");
        assert!(bare.frame.is_empty());
        assert!(!signature_of(&bare).bands.is_empty());
    }

    #[test]
    fn jaccard_is_exact_and_symmetric() {
        let a = tokens("we deploy on tuesdays");
        let b = tokens("we deploy on thursdays");
        let score = jaccard(&a, &b);
        // {we, deploy, on} shared of {we, deploy, on, tuesdays, thursdays}.
        assert!((score - 0.6).abs() < 1e-9, "got {score}");
        assert!((jaccard(&b, &a) - score).abs() < 1e-9);
        assert!((jaccard(&a, &a) - 1.0).abs() < 1e-9);
        assert!(jaccard(&a, &tokens("entirely different words here")) < 0.2);
    }

    /// The property the whole nominator rests on: same content, same bands,
    /// in any process. If this ever changes, existing rows stop colliding
    /// with new candidates and the feature silently stops working.
    #[test]
    fn signatures_are_deterministic_and_correctly_shaped() {
        let a = signature("the deploy runbook lives in docs/deploy.md");
        let b = signature("the deploy runbook lives in docs/deploy.md");
        assert_eq!(a, b, "same content, same signature");
        assert_eq!(a.signature.len(), MINHASH_PERMUTATIONS);
        assert_eq!(a.bands.len(), LSH_BANDS);
        assert_eq!(LSH_BANDS * LSH_ROWS, MINHASH_PERMUTATIONS);
        // Word order is not content: the frames are the same set, so the
        // signatures are too.
        assert_eq!(signature("alpha beta"), signature("beta alpha"));
        assert_ne!(
            signature("alpha beta").bands,
            signature("gamma delta").bands
        );
        // The frame is what is hashed, so a statement and the same statement
        // with its value changed are one bucket — which is the whole point:
        // the pair is nominated, and *scored* on the full token set, where it
        // is nothing like a duplicate.
        let tuesday = tokenise("we deploy on tuesdays");
        let thursday = tokenise("we deploy on thursdays");
        assert_eq!(signature_of(&tuesday), signature_of(&thursday));
        assert!(
            jaccard(&tuesday.all, &thursday.all) < 0.7,
            "same bucket, and emphatically not the same statement"
        );
    }

    /// The LSH promise, on the case the AC depends on: a one-value edit
    /// collides, and two unrelated statements do not. Not a probabilistic
    /// claim about the family — a pin on these exact strings, because these
    /// are the shapes the knowledge-update scenarios use.
    #[test]
    fn near_statements_share_a_band_and_unrelated_ones_do_not() {
        let shares = |a: &str, b: &str| {
            let left = signature(a);
            let right = signature(b);
            left.bands.iter().any(|band| right.bands.contains(band))
        };
        assert!(
            shares("we deploy on tuesdays", "we deploy on thursdays"),
            "the canonical knowledge update must be nominated"
        );
        assert!(shares(
            "the stand-up is at 09:30 every weekday",
            "the stand-up moved to 10:15 every weekday"
        ));
        assert!(shares(
            "i prefer dark roast coffee in the morning",
            "i now prefer light roast coffee in the morning"
        ));
        assert!(
            !shares(
                "we deploy on tuesdays",
                "the incident review is owned by the payments team"
            ),
            "unrelated statements must not cost a nomination slot"
        );
    }

    /// A near-duplicate the *semantic* leg would have to catch on its own if
    /// the bands missed it — recorded as a test so the miss is visible rather
    /// than assumed away. A full re-wording shares no frame words, which is
    /// exactly why ADR-0039 decision 2 keeps two legs.
    #[test]
    fn a_full_rewording_is_the_lexical_legs_known_blind_spot() {
        let a = signature("deployment happens every tuesday");
        let b = signature("we ship code at the start of each week");
        assert!(
            !a.bands.iter().any(|band| b.bands.contains(band)),
            "if this ever passes, the dense leg has less to do, not more"
        );
    }
}
