use super::report::POLICED_ROOT;
use super::scan::DecodeSite;

/// A decode exempt from the guard, keyed by (file, function, target, what) — all
/// reflow-stable, none positional — plus how many identical sites that key covers.
///
/// **The count is load-bearing, not decoration.** `sqlx-newtype-bind`'s substring
/// needles exempt "every matching line under the policed root, not one site" (its own
/// doc says so), which is a region-scoped exemption: a new violation inside the reach
/// passes silently. The population here really does contain byte-identical decode
/// pairs that no key can separate — two `COUNT(*) FROM {table}` calls in one `match`,
/// two `query_scalar(sql)` arms in one helper. Declaring the multiplicity means
/// gaining a third is a mismatch and a failure, not a silent absorption.
pub(super) struct Allowed {
    /// Path suffix under [`POLICED_ROOT`], e.g. `backup.rs` or `sqlite/mod.rs`.
    pub(super) file: &'static str,
    /// Enclosing function name.
    pub(super) function: &'static str,
    /// Rendered decode target, whitespace-stripped, e.g. `i64` or `Option<i64>`.
    pub(super) target: &'static str,
    /// Rendered first argument of the decode call, whitespace-stripped — the SQL
    /// literal, the column name, or the expression that produced it. A **key only**:
    /// nothing in the rule branches on it.
    pub(super) what: &'static str,
    /// How many identical decodes this entry covers.
    pub(super) count: usize,
    /// What kind of exemption this is. Grouping only — see [`Category`].
    pub(super) category: Category,
    /// Why this decode legitimately yields a primitive.
    pub(super) reason: &'static str,
}

/// What kind of exemption an [`Allowed`] entry is, so the failure output can be read by
/// rationale instead of by file.
///
/// **Nothing in the matching rule or the count check branches on this.** It exists because
/// the allowlist's whole value is that a human reads it, and a flat list where a third of
/// the entries are variations of "a name out of `information_schema`" is a list people skim
/// — which is how a region exemption sneaks in wearing a dozen costumes. An enum rather
/// than a string so a typo is a compile error and the grouping order is total.
///
/// [`Category::DeferredNewtype`] is the one that carries an obligation: it means "this
/// *should* be a domain type and is not yet", so its reason must name a tracking issue —
/// enforced in [`problems`]. That makes the allowlist a worklist rather than a graveyard:
/// `DeferredNewtype` entries are the remaining unwrapped storage values, and the gate's own
/// staleness check deletes each one as it gets typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Category {
    /// `COUNT(*)`, `SELECT EXISTS(…)`, and other cardinality probes.
    CountOrExists,
    /// A flag or counter whose primitive **is** the whole domain — a `bool` with two
    /// meaningful states, an integer retry count. A newtype would wrap nothing.
    ///
    /// Distinct from [`Category::CountOrExists`], which is about a *query shape*: reading
    /// `email_verified` is not a cardinality probe, and filing it under one would dilute
    /// exactly the by-rationale reading these categories exist for.
    FlagOrCounter,
    /// Names and versions read out of the database's own catalog.
    SchemaIntrospection,
    /// A blob the storage layer deliberately does not interpret — raw JSON, a cached
    /// response body.
    OpaquePayload,
    /// A value that is *deliberately* stored lossily, so the domain type would claim more
    /// than the column holds.
    DeliberateLossy,
    /// Not an sqlx decode at all — the gate's population is defined structurally, so it
    /// reaches a few constructs that are not row reads. See the module doc's over-bite
    /// note.
    NotADecodeTarget,
    /// Test scaffolding whose type comes from a generic helper's signature.
    TestScaffolding,
    /// A query target whose hand-written `FromRow` decoder cannot satisfy the narrow
    /// direct-column-get proof. Each exact target is listed here.
    ///
    /// A decoder that satisfies the proof is approved by delegation, while the scanner
    /// still polices each direct typed column get in its body.
    HandWrittenFromRow,
    /// **Residue, not a verdict.** This should be a domain type; the fix is a vertical
    /// tracked elsewhere. The reason must name the issue.
    DeferredNewtype,
}

impl Category {
    /// Every variant, so the failure footer can group in a stable, total order without a
    /// `HashMap` iteration order or a hand-kept second list.
    pub(super) const ALL: &'static [Self] = &[
        Self::CountOrExists,
        Self::FlagOrCounter,
        Self::SchemaIntrospection,
        Self::OpaquePayload,
        Self::DeliberateLossy,
        Self::NotADecodeTarget,
        Self::TestScaffolding,
        Self::HandWrittenFromRow,
        Self::DeferredNewtype,
    ];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::CountOrExists => "count-or-exists",
            Self::FlagOrCounter => "flag-or-counter",
            Self::SchemaIntrospection => "schema-introspection",
            Self::OpaquePayload => "opaque-payload",
            Self::DeliberateLossy => "deliberate-lossy",
            Self::NotADecodeTarget => "not-a-decode-target",
            Self::TestScaffolding => "test-scaffolding",
            Self::HandWrittenFromRow => "hand-written-fromrow",
            Self::DeferredNewtype => "deferred-newtype",
        }
    }
}

/// Whether `reason` names a tracking issue (`#` followed by at least one digit).
///
/// Only [`Category::DeferredNewtype`] requires one. A deferred entry whose reason names no
/// issue is a TODO with no owner — the shape that turns an allowlist into a graveyard.
fn names_an_issue(reason: &str) -> bool {
    reason
        .split('#')
        .skip(1)
        .any(|rest| rest.starts_with(|c: char| c.is_ascii_digit()))
}

/// Every decode that is genuinely primitive, each with its reason.
///
/// **No entry here may name a decode that yields a domain value.** That is the whole
/// point: this list is the complete population of legitimate untyped decodes under the root,
/// and anything not on it is a failure.
pub(super) const ALLOWLIST: &[Allowed] = &[
    // ---- schema introspection: names and definitions out of the DB's own catalog ----
    Allowed {
        file: "postgres/backup.rs",
        function: "existing_export_tables",
        target: "String",
        what: "\"table_name\"",
        count: 1,
        category: Category::SchemaIntrospection,
        reason: "a table name from information_schema — a catalog identifier, not a domain value",
    },
    Allowed {
        file: "postgres/backup.rs",
        function: "repair_sequences",
        target: "String",
        what: "\"table_name\"",
        count: 1,
        category: Category::SchemaIntrospection,
        reason: "a table name read from information_schema and spliced back into DDL — a \
                 catalog identifier the domain model has no type for",
    },
    Allowed {
        file: "postgres/backup.rs",
        function: "repair_sequences",
        target: "String",
        what: "\"column_name\"",
        count: 1,
        category: Category::SchemaIntrospection,
        reason: "a column name from information_schema",
    },
    Allowed {
        file: "postgres/backup.rs",
        function: "columns",
        target: "String",
        what: "\"column_name\"",
        count: 1,
        category: Category::SchemaIntrospection,
        reason: "a column name from information_schema, into the plain ColumnInfo struct",
    },
    Allowed {
        file: "postgres/backup.rs",
        function: "columns",
        target: "String",
        what: "\"udt_name\"",
        count: 1,
        category: Category::SchemaIntrospection,
        reason: "a Postgres type name from information_schema",
    },
    Allowed {
        file: "postgres/backup.rs",
        function: "export_table",
        target: "String",
        what: "0",
        count: 1,
        category: Category::OpaquePayload,
        reason: "the row rendered as JSON by the query itself — an opaque payload this layer \
                 never interprets",
    },
    Allowed {
        file: "postgres/backup.rs",
        function: "schema_checksum",
        target: "String",
        what: "\"table_name\"",
        count: 1,
        category: Category::SchemaIntrospection,
        reason: "a table name from information_schema, hashed into the schema fingerprint",
    },
    Allowed {
        file: "postgres/backup.rs",
        function: "schema_checksum",
        target: "String",
        what: "\"column_name\"",
        count: 1,
        category: Category::SchemaIntrospection,
        reason: "a column name from information_schema, hashed into the schema fingerprint",
    },
    Allowed {
        file: "postgres/backup.rs",
        function: "schema_checksum",
        target: "String",
        what: "\"udt_name\"",
        count: 1,
        category: Category::SchemaIntrospection,
        reason: "a Postgres type name from information_schema, hashed into the fingerprint",
    },
    Allowed {
        file: "postgres/backup.rs",
        function: "schema_checksum",
        target: "String",
        what: "\"is_nullable\"",
        count: 1,
        category: Category::SchemaIntrospection,
        reason: "information_schema's YES/NO nullability flag — a catalog string, not a bool \
                 column",
    },
    Allowed {
        file: "sqlite/backup.rs",
        function: "existing_export_tables",
        target: "String",
        what: "\"name\"",
        count: 1,
        category: Category::SchemaIntrospection,
        reason: "a table name from sqlite_master, the dialect twin of the Postgres read",
    },
    Allowed {
        file: "sqlite/backup.rs",
        function: "columns",
        target: "String",
        what: "\"name\"",
        count: 1,
        category: Category::SchemaIntrospection,
        reason: "a column name from PRAGMA table_info, into the plain ColumnInfo struct",
    },
    Allowed {
        file: "sqlite/backup.rs",
        function: "columns",
        target: "String",
        what: "\"type\"",
        count: 1,
        category: Category::SchemaIntrospection,
        reason: "a SQLite declared column type from PRAGMA table_info",
    },
    Allowed {
        file: "backup/restore_validation.rs",
        function: "backed_up_schema_columns",
        target: "String",
        what: "\"SELECTnameFROMsqlite_masterWHEREtype='table'\"",
        count: 1,
        category: Category::SchemaIntrospection,
        reason: "test-only live schema table inventory used to prove restore typed-column \
                 coverage is tied to the backed-up table surface",
    },
    Allowed {
        file: "backup/restore_validation.rs",
        function: "backed_up_schema_columns",
        target: "String",
        what: "\"name\"",
        count: 1,
        category: Category::SchemaIntrospection,
        reason: "test-only PRAGMA table_info column name used to prove restore typed-column \
                 coverage entries exist in the live backed-up schema",
    },
    Allowed {
        file: "sqlite/backup.rs",
        function: "export_table",
        target: "String",
        what: "0",
        count: 1,
        category: Category::OpaquePayload,
        reason: "the row rendered as JSON by the query itself, the twin of the Postgres export",
    },
    Allowed {
        file: "sqlite/backup.rs",
        function: "schema_checksum",
        target: "String",
        what: "\"name\"",
        count: 1,
        category: Category::SchemaIntrospection,
        reason: "an object name from sqlite_master, hashed into the schema fingerprint",
    },
    Allowed {
        file: "sqlite/backup.rs",
        function: "schema_checksum",
        target: "String",
        what: "\"sql\"",
        count: 1,
        category: Category::SchemaIntrospection,
        reason: "the stored DDL text from sqlite_master, hashed into the schema fingerprint",
    },
    Allowed {
        file: "postgres/open.rs",
        function: "database_is_empty",
        target: "String",
        what: "\"SELECTtable_nameFROMinformation_schema.tables\\WHEREtable_schema='public'ANDtable_type='BASETABLE'\\ANDtable_name<>'_sqlx_migrations'\"",
        count: 1,
        category: Category::SchemaIntrospection,
        reason: "table names enumerated to decide emptiness",
    },
    Allowed {
        file: "sqlite/open.rs",
        function: "database_is_empty",
        target: "String",
        what: "\"SELECTnameFROMsqlite_master\\WHEREtype='table'ANDnameNOTLIKE'sqlite_%'ANDname<>'_sqlx_migrations'\"",
        count: 1,
        category: Category::SchemaIntrospection,
        reason: "table names enumerated to decide emptiness, the SQLite twin",
    },
    // ---- cardinality probes ----
    Allowed {
        file: "postgres/open.rs",
        function: "database_is_empty",
        target: "bool",
        what: "&format!(\"SELECTEXISTS(SELECT1FROM{}LIMIT1)\",crate::sql::quote_identifier(&table))",
        count: 1,
        category: Category::CountOrExists,
        reason: "SELECT EXISTS(…) emptiness probe; the SQLite twin decodes i64 (no bool there)",
    },
    Allowed {
        file: "test_support.rs",
        function: "raw_media_filename_exists",
        target: "i64",
        what: "\"SELECTEXISTS(SELECT1FROMmediaWHEREfilename=$1)\"",
        count: 1,
        category: Category::CountOrExists,
        reason: "SQLite existence probe for intentionally invalid backup filename fixture",
    },
    Allowed {
        file: "test_support.rs",
        function: "raw_media_filename_exists",
        target: "bool",
        what: "\"SELECTEXISTS(SELECT1FROMmediaWHEREfilename=$1)\"",
        count: 1,
        category: Category::CountOrExists,
        reason: "Postgres existence probe for intentionally invalid backup filename fixture",
    },
    Allowed {
        file: "sqlite/posts.rs",
        function: "set_post_tags",
        target: "bool",
        what: "\"SELECTCOUNT(*)>0FROMpostsWHEREpost_id=$1\"",
        count: 1,
        category: Category::CountOrExists,
        reason: "post-exists check before the batched tag write; Postgres uses a FOR UPDATE probe instead",
    },
    Allowed {
        file: "posts.rs",
        function: "physical_row_ids",
        target: "String",
        what: "\"SELECTctid::textFROMpost_tagsWHEREpost_id=$1ORDERBYtag_id\"",
        count: 1,
        category: Category::TestScaffolding,
        reason: "physical row identity, not a domain value: set_post_tags must leave unchanged tag rows untouched (#771), and column values cannot show that — a DELETE+INSERT reproduces them exactly. Postgres arm",
    },
    Allowed {
        file: "posts.rs",
        function: "physical_row_ids",
        target: "String",
        what: "\"SELECTCAST(rowidASTEXT)FROMpost_tagsWHEREpost_id=$1ORDERBYtag_id\"",
        count: 1,
        category: Category::TestScaffolding,
        reason: "physical row identity, not a domain value: the SQLite twin of the ctid probe above (#771)",
    },
    Allowed {
        file: "posts.rs",
        function: "list_posts_by_tag",
        target: "bool",
        what: "\"SELECTCOUNT(*)>0FROMtagsWHEREtag_slug=$1\"",
        count: 1,
        category: Category::CountOrExists,
        reason: "tag-existence check, so an unknown tag is a 404 rather than an empty list",
    },
    Allowed {
        file: "posts.rs",
        function: "list_user_posts_by_tag",
        target: "bool",
        what: "\"SELECTCOUNT(*)>0FROMtagsWHEREtag_slug=$1\"",
        count: 1,
        category: Category::CountOrExists,
        reason: "the same tag-existence check on the per-user listing",
    },
    Allowed {
        file: "postgres/teardown.rs",
        function: "database_exists",
        target: "bool",
        what: "\"SELECTEXISTS(SELECT1FROMpg_databaseWHEREdatname=$1)\"",
        count: 1,
        category: Category::CountOrExists,
        reason: "database-exists probe before a teardown DROP",
    },
    Allowed {
        file: "postgres/feed_events.rs",
        function: "claimable_count",
        target: "i64",
        what: "\"SELECTCOUNT(*)FROMfeed_events\\WHERE(status='pending'ANDnext_attempt_at<=$1)\\OR(status='claimed'ANDclaimed_at<$2)\"",
        count: 1,
        category: Category::CountOrExists,
        reason: "COUNT(*) of claimable feed-event work items; a queue depth, not a row identity",
    },
    Allowed {
        file: "sqlite/feed_events.rs",
        function: "claimable_count",
        target: "i64",
        what: "\"SELECTCOUNT(*)FROMfeed_events\\WHERE(status='pending'ANDnext_attempt_at<=$1)\\OR(status='claimed'ANDclaimed_at<$2)\"",
        count: 1,
        category: Category::CountOrExists,
        reason: "COUNT(*) of claimable feed-event work items; the SQLite twin",
    },
    // ---- deliberately lossy / opaque ----
    Allowed {
        file: "helpers.rs",
        function: "",
        target: "String",
        what: "label",
        count: 1,
        category: Category::DeliberateLossy,
        reason: "the session label is stored lossily (SessionLabel::from_lossy truncates), so \
                 the column holds less than the domain type claims — decoding into it would \
                 assert an invariant the data does not carry (#728 names this site explicitly)",
    },
    Allowed {
        file: "feed_cache.rs",
        function: "",
        target: "String",
        what: "body",
        count: 1,
        category: Category::OpaquePayload,
        reason: "the cached feed body — rendered RSS/Atom/JSON this layer stores and serves \
                 verbatim, never inspects. Note the same tuple's feed_url and content_type \
                 DO decode into FeedPath/ContentType",
    },
    Allowed {
        file: "backup/restore_validation.rs",
        function: "restore_text",
        target: "Option<RestoreText>",
        what: "column",
        count: 1,
        category: Category::OpaquePayload,
        reason: "restore validation reads an NDJSON cell selected by table-specific row \
                 validator code, then immediately parses it through the typed restore-row \
                 domain field; this is not a sqlx row decode",
    },
    Allowed {
        file: "posts.rs",
        function: "from_row",
        target: "String",
        what: "\"tags\"",
        count: 1,
        category: Category::OpaquePayload,
        reason: "the tags JSON aggregate built by TAGS_SUBQUERY: aggregate semantics require \
                 parsing its text after the post id is available, rather than decoding one \
                 domain column directly",
    },
    Allowed {
        file: "feed_events.rs",
        function: "",
        target: "Option<String>",
        what: "last_error",
        count: 1,
        category: Category::OpaquePayload,
        reason: "free-text error detail from a failed regeneration attempt; no shape to type",
    },
    Allowed {
        file: "feed_events.rs",
        function: "",
        target: "i32",
        what: "attempts",
        count: 1,
        category: Category::FlagOrCounter,
        reason: "retry counter for the claim-lease backoff — an integer compared against a \
                 max, with no identity of its own",
    },
    // ---- flags on row tuples ----
    Allowed {
        file: "helpers.rs",
        function: "",
        target: "bool",
        what: "UserRow.7",
        count: 1,
        category: Category::FlagOrCounter,
        reason: "email_verified — a two-state flag whose meaning is exhausted by the bool; \
                 there is no wider domain for a newtype to carry",
    },
    Allowed {
        file: "helpers.rs",
        function: "",
        target: "bool",
        what: "UserRow.8",
        count: 1,
        category: Category::FlagOrCounter,
        reason: "is_operator — the same two-state shape; the authorization *decision* is a \
                 domain concept, but the stored bit is not",
    },
    Allowed {
        file: "users.rs",
        function: "authenticate_with",
        target: "(UserId,Username,Option<DisplayName>,Option<Bio>,UtcInstant,Option<UtcInstant>,StoredPasswordHash,Option<Email>,bool,bool,)",
        count: 1,
        what: "\"SELECTuser_id,username,display_name,bio,created_at,last_authenticated_at,password_hash,email,email_verified,is_operatorFROMusersWHEREusername=$1\"",
        category: Category::FlagOrCounter,
        reason: "email_verified and is_operator — the same two-state flags the helpers.rs \
                 entries describe, and the only unapproved leaves left here. The \
                 password_hash element was this entry's deferred-newtype residue (#693) and \
                 is now StoredPasswordHash, decoding through its bridge",
    },
    // ---- config values: #687 owns the key half, nothing owns the value half ----
    Allowed {
        file: "site_config.rs",
        function: "get_raw",
        target: "(String,)",
        what: "\"SELECTvalueFROMsite_configWHEREkey=$1\"",
        count: 1,
        category: Category::OpaquePayload,
        reason: "a site-config value is deliberately polymorphic text (a URL, a port, a token) \
                 parsed by each key's own getter. #687 types the KEY half; this entry \
                 survives it, because the value half stays String by design",
    },
    Allowed {
        file: "site_config.rs",
        function: "",
        target: "String",
        what: "SiteConfigExportRow.0",
        count: 1,
        category: Category::DeferredNewtype,
        reason: "the key half of SiteConfigExportRow. It survives #687 permanently, by design: \
                 `list` is a faithful dump of what is physically stored, so it must be able to \
                 return a row whose key is NOT in the registry — a legacy or hand-written \
                 orphan. Typing this element would make those rows undecodable, i.e. invisible, \
                 which is the one thing an operator debugging a stale value cannot afford \
                 (#687 D4). The CLI parses and judges them instead",
    },
    Allowed {
        file: "site_config.rs",
        function: "",
        target: "String",
        what: "SiteConfigExportRow.1",
        count: 1,
        category: Category::OpaquePayload,
        reason: "the value half of SiteConfigExportRow is deliberately polymorphic text (a URL, \
                 a port, a token) parsed by each key's own getter; it stays String by design",
    },
    Allowed {
        file: "site_config.rs",
        function: "delete",
        target: "(String,)",
        what: "\"DELETEFROMsite_configWHEREkey=$1RETURNINGkey\"",
        count: 1,
        category: Category::DeferredNewtype,
        reason: "the RETURNING key echoes back the config key — #687's SiteConfigKey territory",
    },
    Allowed {
        file: "user_config.rs",
        function: "get",
        target: "(String,)",
        what: "\"SELECTvalueFROMuser_configWHEREuser_id=$1ANDkey=$2\"",
        count: 1,
        category: Category::OpaquePayload,
        reason: "a per-user config value, polymorphic text like its site-config sibling",
    },
    // ---- the claim wrapper ----
    Allowed {
        file: "postgres/feed_events.rs",
        function: "claim_pending_batch",
        target: "ClaimedRow",
        what: "\"WITHeligibleAS(\\SELECTidFROMfeed_events\\WHERE(status='pending'ANDnext_attempt_at<=$1)\\OR(status='claimed'ANDclaimed_at<$2)\\ORDERBYnext_attempt_atASC\\LIMIT$3\\FORUPDATESKIPLOCKED\\)\\UPDATEfeed_eventsSETstatus='claimed',claimed_at=$1\\WHEREidIN(SELECTidFROMeligible)\\RETURNINGid,feed_url,status,attempts,last_error,next_attempt_at,claimed_at,\\created_at,regenerated_at,pinged_at\"",
        count: 1,
        category: Category::HandWrittenFromRow,
        reason: "ClaimedRow's FromRow is hand-written (it must divert a corrupt feed_url to \
                 the purge list), so delegation cannot back it. Its parts are accounted for: \
                 FeedEventRecord is a policed FromRow struct and FeedEventId is a bridge type",
    },
    Allowed {
        file: "sqlite/feed_events.rs",
        function: "claim_pending_batch",
        target: "ClaimedRow",
        what: "\"UPDATEfeed_eventsSETstatus='claimed',claimed_at=$1\\WHEREidIN(\\SELECTidFROMfeed_events\\WHERE(status='pending'ANDnext_attempt_at<=$2)\\OR(status='claimed'ANDclaimed_at<$3)\\ORDERBYnext_attempt_atASC\\LIMIT$4\\)\\RETURNINGid,feed_url,status,attempts,last_error,next_attempt_at,claimed_at,\\created_at,regenerated_at,pinged_at\"",
        count: 1,
        category: Category::HandWrittenFromRow,
        reason: "the dialect twin of the Postgres claim; same wrapper, same accounting",
    },
    // ---- test scaffolding ----
    Allowed {
        file: "test_support.rs",
        function: "ensure_template_db",
        target: "bool",
        what: "\"SELECTEXISTS(SELECT1FROMpg_databaseWHEREdatname=$1)\"",
        count: 1,
        category: Category::CountOrExists,
        reason: "database-exists probe in the Postgres test harness",
    },
    Allowed {
        file: "sqlite/open.rs",
        function: "database_is_empty",
        target: "i64",
        what: "&format!(\"SELECTEXISTS(SELECT1FROM{}LIMIT1)\",crate::sql::quote_identifier(&table))",
        count: 1,
        category: Category::CountOrExists,
        reason: "SELECT EXISTS(…) decoded as i64 — SQLite has no bool; the Postgres twin decodes bool",
    },
    Allowed {
        file: "postgres/schema.rs",
        function: "every_foreign_key_is_deferrable",
        target: "i64",
        what: "\"SELECTCOUNT(*)FROMpg_constraint\\WHEREcontype='f'ANDconnamespace='public'::regnamespace\\ANDNOTcondeferrable\"",
        count: 1,
        category: Category::CountOrExists,
        reason: "COUNT(*) of non-deferrable FK constraints",
    },
    Allowed {
        file: "postgres/backup.rs",
        function: "schema_version",
        target: "Option<i64>",
        what: "\"SELECTMAX(version)FROM_sqlx_migrations\"",
        count: 1,
        category: Category::SchemaIntrospection,
        reason: "MAX(version) migration version, NULL on an empty migrations table",
    },
    Allowed {
        file: "sqlite/backup.rs",
        function: "schema_version",
        target: "Option<i64>",
        what: "\"SELECTMAX(version)FROM_sqlx_migrations\"",
        count: 1,
        category: Category::SchemaIntrospection,
        reason: "MAX(version) migration version, the dialect twin of the Postgres one",
    },
    Allowed {
        file: "subscriptions.rs",
        function: "is_subscriber",
        target: "(i64,)",
        what: "DB::IS_ACTIVE_SUBSCRIBER",
        count: 1,
        category: Category::CountOrExists,
        reason: "Existence flag, not an id — subscriptions.rs's own bound comment says so",
    },
    Allowed {
        file: "subscriptions.rs",
        function: "is_subscriber",
        target: "(i64,)",
        what: "DB::IS_ACTIVE_LOCAL_SUBSCRIBER",
        count: 1,
        category: Category::CountOrExists,
        reason: "Existence flag, not an id — the local-channel twin of IS_ACTIVE_SUBSCRIBER",
    },
];

/// Whether `path` is exactly `POLICED_ROOT/relative`.
///
/// **Exact, not a suffix match.** Three files under the root are named `backup.rs`,
/// and two of them declare a function called `schema_version`, so a suffix match would
/// let the `backup.rs` entry reach decodes in `postgres/backup.rs` — a region-scoped
/// exemption of exactly the kind the site-scoping rule exists to stop. The other key
/// fields happen to keep it honest today; that is luck, not design.
fn file_matches(path: &str, relative: &str) -> bool {
    path.strip_prefix(POLICED_ROOT)
        .map(|rest| rest.trim_start_matches('/'))
        == Some(relative)
}

/// Whether `entry` names `decode` in `path`.
pub(super) fn entry_matches(entry: &Allowed, path: &str, decode: &DecodeSite) -> bool {
    file_matches(path, entry.file)
        && entry.function == decode.function
        && entry.target == decode.target
        && entry.what == decode.what
}

/// Faults in an allowlist itself, independent of the tree.
///
/// A gate that polices the source but not its own exemption list is blind in the one place
/// it can least afford to be — the same rule as failing on an unparseable file, applied
/// inward (ADR-0085 principle 6).
///
/// Takes the list rather than reading [`ALLOWLIST`] directly so the tests drive *this*
/// function with synthetic entries instead of re-implementing the rule beside it.
pub(super) fn allowlist_self_problems(allowlist: &[Allowed]) -> Vec<String> {
    let mut lines = Vec::new();

    // Duplicate keys. Matching is `.any(…)` and the count check is per-entry, so two
    // entries with the same key each declaring 1 would BOTH pass while double-covering a
    // single site — and deleting the decode would then need two edits to go green, which
    // is exactly how a stale exemption survives.
    for (i, a) in allowlist.iter().enumerate() {
        if let Some(dup) = allowlist[..i].iter().find(|b| {
            (b.file, b.function, b.target, b.what) == (a.file, a.function, a.target, a.what)
        }) {
            lines.push(format!(
                "{}::{} `{}`: two allowlist entries share one key ({} and {}). Merge them into \
                 one entry and state the combined multiplicity in `count` — two entries covering \
                 one site can never go stale together.",
                a.file, a.function, a.target, dup.reason, a.reason
            ));
        }
    }

    // A deferred entry with no issue is a TODO with no owner.
    for a in allowlist {
        if a.category == Category::DeferredNewtype && !names_an_issue(a.reason) {
            lines.push(format!(
                "{}::{} `{}`: a `deferred-newtype` entry must name the issue tracking the fix \
                 (e.g. \"…, deferred to #750\"). Without one this is not deferred work, it is an \
                 exemption with a sympathetic label.",
                a.file, a.function, a.target
            ));
        }
    }

    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An entry with everything but the field under test held fixed.
    fn entry(what: &'static str, category: Category, reason: &'static str) -> Allowed {
        Allowed {
            file: "users.rs",
            function: "f",
            target: "i64",
            what,
            count: 1,
            category,
            reason,
        }
    }

    #[test]
    fn the_category_field_changes_nothing_about_matching_or_counting() {
        // A8's falsifiable form. "No code path branches on `category`" cannot be asserted
        // from a test, but its observable consequence can: two entries that differ ONLY in
        // category must behave identically for both the match and the duplicate check.
        let a = entry("\"SELECTCOUNT(*)\"", Category::CountOrExists, "a count");
        let b = entry("\"SELECTCOUNT(*)\"", Category::OpaquePayload, "a count");
        let site = DecodeSite {
            function: "f".to_string(),
            target: "i64".to_string(),
            what: "\"SELECTCOUNT(*)\"".to_string(),
            unapproved: vec!["i64".to_string()],
            line: 1,
        };
        let path = "storage/src/users.rs";
        assert_eq!(
            entry_matches(&a, path, &site),
            entry_matches(&b, path, &site),
            "category must not affect whether an entry covers a site"
        );
        // …and differing only in category does NOT make two entries distinct, so it cannot
        // be used to sneak a second entry past the duplicate check.
        assert_eq!(
            allowlist_self_problems(&[a, b]).len(),
            1,
            "same key, still a duplicate"
        );
    }

    #[test]
    fn category_drives_only_the_deferred_obligation() {
        // The precise claim: `category` is inert for matching and counting, but it is NOT
        // decoration — `DeferredNewtype` alone carries the name-your-issue obligation.
        let ok = entry("\"a\"", Category::CountOrExists, "a count");
        assert!(allowlist_self_problems(std::slice::from_ref(&ok)).is_empty());
        let deferred = entry("\"a\"", Category::DeferredNewtype, "should be a newtype");
        assert_eq!(
            allowlist_self_problems(std::slice::from_ref(&deferred)).len(),
            1,
            "a deferred entry naming no issue must fail"
        );
    }

    #[test]
    fn two_entries_with_one_key_are_a_failure() {
        let a = entry("\"SELECTCOUNT(*)\"", Category::CountOrExists, "first");
        let b = entry("\"SELECTCOUNT(*)\"", Category::CountOrExists, "second");
        assert_eq!(allowlist_self_problems(&[a, b]).len(), 1);
    }

    #[test]
    fn distinct_keys_are_not_duplicates() {
        let a = entry("\"SELECTCOUNT(*)\"", Category::CountOrExists, "first");
        let b = entry("\"SELECTMAX(v)\"", Category::CountOrExists, "second");
        assert!(allowlist_self_problems(&[a, b]).is_empty());
    }

    #[test]
    fn the_shipped_allowlist_has_no_self_faults() {
        // The self-checks run on every gate invocation, so a bad entry would fail the gate
        // on a clean tree. Pin it here too, where the message is about the allowlist rather
        // than about whatever else was failing.
        assert!(
            allowlist_self_problems(ALLOWLIST).is_empty(),
            "{:?}",
            allowlist_self_problems(ALLOWLIST)
        );
    }

    #[test]
    fn a_deferred_newtype_entry_must_name_its_issue() {
        assert!(names_an_issue(
            "subscriber_ref should be a newtype, deferred to #750"
        ));
        assert!(!names_an_issue(
            "subscriber_ref should be a newtype one day"
        ));
        // A bare `#` with no number is a false positive waiting to happen — a reason that
        // says "the # column" must not count as a tracking reference.
        assert!(!names_an_issue("the # column is opaque"));
    }
    #[test]
    fn an_entry_does_not_reach_a_same_named_file_in_a_subdirectory() {
        // `backup.rs`, `postgres/backup.rs` and `sqlite/backup.rs` all exist, and the
        // last two both declare `schema_version`. A suffix match would let one entry
        // exempt decodes in a sibling file — a region exemption by the back door.
        assert!(file_matches("storage/src/backup.rs", "backup.rs"));
        assert!(!file_matches("storage/src/postgres/backup.rs", "backup.rs"));
        assert!(file_matches(
            "storage/src/postgres/backup.rs",
            "postgres/backup.rs"
        ));
        assert!(!file_matches("storage/src/sqlite/mod.rs", "mod.rs"));
    }
}
