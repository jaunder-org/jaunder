-- Additive typed aggregate for custom public themes. Legacy config remains readable.
CREATE TABLE themes (
    id INTEGER PRIMARY KEY,
    catalog_owner_key TEXT NOT NULL,
    name TEXT NOT NULL,
    name_key TEXT NOT NULL,
    current_revision_digest TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CHECK (name_key NOT IN ('terminal', 'studio', 'reader')),
    UNIQUE (id, catalog_owner_key),
    UNIQUE (catalog_owner_key, name_key),
    FOREIGN KEY (id, current_revision_digest)
        REFERENCES theme_revisions(theme_id, digest)
        DEFERRABLE INITIALLY DEFERRED
);
CREATE TABLE theme_drafts (
    theme_id INTEGER PRIMARY KEY
        REFERENCES themes(id) ON DELETE CASCADE,
    manifest BLOB NOT NULL,
    stylesheet BLOB NOT NULL,
    source_digest TEXT NOT NULL CHECK (length(source_digest) = 64)
);
CREATE TABLE theme_revisions (
    id INTEGER PRIMARY KEY,
    theme_id INTEGER NOT NULL REFERENCES themes(id) ON DELETE CASCADE,
    digest TEXT NOT NULL CHECK (length(digest) = 64),
    stylesheet_digest TEXT NOT NULL CHECK (length(stylesheet_digest) = 64),
    manifest BLOB NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE (theme_id, digest)
);
CREATE INDEX theme_revisions_theme_id_created_at
    ON theme_revisions(theme_id, created_at DESC);
CREATE TABLE theme_revision_assets (
    theme_id INTEGER NOT NULL,
    revision_digest TEXT NOT NULL,
    path TEXT NOT NULL,
    digest TEXT NOT NULL CHECK (length(digest) = 64),
    mime TEXT NOT NULL,
    PRIMARY KEY (theme_id, revision_digest, path),
    FOREIGN KEY (theme_id, revision_digest)
        REFERENCES theme_revisions(theme_id, digest)
        ON DELETE CASCADE
);
CREATE TABLE theme_content_eligibility (
    digest TEXT PRIMARY KEY CHECK (length(digest) = 64),
    mime TEXT NOT NULL,
    retained_until_unix_seconds INTEGER NOT NULL,
    live_references INTEGER NOT NULL DEFAULT 0 CHECK (live_references >= 0)
);
CREATE TABLE theme_retained_content_charges (
    catalog_owner_key TEXT NOT NULL,
    digest TEXT NOT NULL CHECK (length(digest) = 64),
    logical_bytes INTEGER NOT NULL CHECK (logical_bytes >= 0),
    physical_bytes INTEGER NOT NULL CHECK (physical_bytes >= 0),
    live_references INTEGER NOT NULL DEFAULT 0 CHECK (live_references >= 0),
    PRIMARY KEY (catalog_owner_key, digest),
    FOREIGN KEY (digest)
        REFERENCES theme_content_eligibility(digest)
        ON DELETE RESTRICT
);
CREATE TABLE theme_role_bindings (
    theme_id INTEGER NOT NULL REFERENCES themes(id) ON DELETE CASCADE,
    role TEXT NOT NULL CHECK (role IN ('logo', 'header')),
    mode TEXT NOT NULL CHECK (mode IN ('packaged_default', 'explicit_absent', 'package_asset', 'media', 'pool')),
    package_path TEXT,
    media_user_id INTEGER,
    media_source TEXT,
    media_digest TEXT,
    media_filename TEXT,
    pool_revision_digest TEXT CHECK (pool_revision_digest IS NULL OR length(pool_revision_digest) = 64),
    shuffle_seed BLOB,
    PRIMARY KEY (theme_id, role),
    FOREIGN KEY (media_user_id, media_digest, media_filename, media_source)
        REFERENCES media(user_id, sha256, filename, source),
    CHECK (
        (mode IN ('packaged_default', 'explicit_absent')
            AND package_path IS NULL
            AND media_user_id IS NULL
            AND media_source IS NULL
            AND media_digest IS NULL
            AND media_filename IS NULL
            AND pool_revision_digest IS NULL
            AND shuffle_seed IS NULL)
        OR (mode = 'package_asset'
            AND package_path IS NOT NULL
            AND media_user_id IS NULL
            AND media_source IS NULL
            AND media_digest IS NULL
            AND media_filename IS NULL
            AND pool_revision_digest IS NULL
            AND shuffle_seed IS NULL)
        OR (mode = 'media'
            AND package_path IS NULL
            AND media_user_id IS NOT NULL
            AND media_source IS NOT NULL
            AND media_digest IS NOT NULL
            AND media_filename IS NOT NULL
            AND pool_revision_digest IS NULL
            AND shuffle_seed IS NULL)
        OR (mode = 'pool'
            AND role = 'header'
            AND package_path IS NULL
            AND media_user_id IS NULL
            AND media_source IS NULL
            AND media_digest IS NULL
            AND media_filename IS NULL
            AND pool_revision_digest IS NOT NULL
            AND shuffle_seed IS NOT NULL
            AND length(shuffle_seed) = 32)
    )
);
CREATE TABLE theme_header_pool (
    theme_id INTEGER NOT NULL REFERENCES themes(id) ON DELETE CASCADE,
    entry_ordinal INTEGER NOT NULL CHECK (entry_ordinal >= 0),
    package_path TEXT,
    media_user_id INTEGER,
    media_source TEXT,
    media_digest TEXT,
    media_filename TEXT,
    PRIMARY KEY (theme_id, entry_ordinal),
    FOREIGN KEY (media_user_id, media_digest, media_filename, media_source)
        REFERENCES media(user_id, sha256, filename, source),
    CHECK (
        (package_path IS NOT NULL
            AND media_user_id IS NULL
            AND media_source IS NULL
            AND media_digest IS NULL
            AND media_filename IS NULL)
        OR (package_path IS NULL
            AND media_user_id IS NOT NULL
            AND media_source IS NOT NULL
            AND media_digest IS NOT NULL
            AND media_filename IS NOT NULL)
    )
);
CREATE TABLE theme_selections (
    catalog_owner_key TEXT PRIMARY KEY,
    builtin_theme TEXT,
    theme_id INTEGER,
    CHECK ((builtin_theme IS NULL) <> (theme_id IS NULL)),
    CHECK (builtin_theme IS NULL OR builtin_theme IN ('terminal', 'studio', 'reader')),
    FOREIGN KEY (theme_id, catalog_owner_key)
        REFERENCES themes(id, catalog_owner_key)
        ON DELETE RESTRICT
);
CREATE TABLE theme_owner_quotas (
    catalog_owner_key TEXT PRIMARY KEY,
    active_themes INTEGER NOT NULL DEFAULT 0 CHECK (active_themes >= 0),
    retained_revisions INTEGER NOT NULL DEFAULT 0 CHECK (retained_revisions >= 0),
    logical_bytes INTEGER NOT NULL DEFAULT 0 CHECK (logical_bytes >= 0)
);
CREATE TABLE theme_site_quota (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    retained_revisions INTEGER NOT NULL DEFAULT 0 CHECK (retained_revisions >= 0),
    physical_bytes INTEGER NOT NULL DEFAULT 0 CHECK (physical_bytes >= 0)
);
INSERT INTO theme_site_quota(singleton)
VALUES (1);
-- #21 selection tokens are copied, not removed; Task 5 owns the cutover.
INSERT INTO theme_selections(catalog_owner_key, builtin_theme, theme_id)
SELECT 'site', value, NULL
FROM site_config
WHERE key = 'site.theme'
    AND value IN ('terminal', 'studio', 'reader');
INSERT INTO theme_selections(catalog_owner_key, builtin_theme, theme_id)
SELECT 'user:' || user_id, value, NULL
FROM user_config
WHERE key = 'user.theme'
    AND value IN ('terminal', 'studio', 'reader');
