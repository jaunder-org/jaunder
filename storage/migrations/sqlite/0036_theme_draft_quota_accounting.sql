-- Mutable drafts have independent references from retained published content.
CREATE TABLE theme_draft_charges (
    theme_id INTEGER PRIMARY KEY REFERENCES themes(id) ON DELETE CASCADE,
    source_digest TEXT NOT NULL CHECK (length(source_digest) = 64),
    logical_bytes INTEGER NOT NULL CHECK (logical_bytes >= 0),
    physical_bytes INTEGER NOT NULL CHECK (physical_bytes >= 0)
);
CREATE TABLE theme_draft_content_charges (
    source_digest TEXT PRIMARY KEY CHECK (length(source_digest) = 64),
    physical_bytes INTEGER NOT NULL CHECK (physical_bytes >= 0),
    live_references INTEGER NOT NULL CHECK (live_references > 0)
);
INSERT INTO theme_draft_charges(theme_id, source_digest, logical_bytes, physical_bytes)
SELECT draft.theme_id, draft.source_digest,
       length(draft.manifest) + length(draft.stylesheet) + COALESCE((SELECT SUM(length(asset.bytes)) FROM theme_draft_assets asset WHERE asset.theme_id = draft.theme_id), 0),
       length(draft.manifest) + length(draft.stylesheet) + COALESCE((SELECT SUM(length(asset.bytes)) FROM theme_draft_assets asset WHERE asset.theme_id = draft.theme_id), 0)
FROM theme_drafts draft;
INSERT INTO theme_draft_content_charges(source_digest, physical_bytes, live_references)
SELECT source_digest, MAX(physical_bytes), COUNT(*) FROM theme_draft_charges GROUP BY source_digest;
UPDATE theme_owner_quotas SET logical_bytes = logical_bytes + COALESCE((SELECT SUM(charge.logical_bytes) FROM theme_draft_charges charge JOIN themes theme ON theme.id = charge.theme_id WHERE theme.catalog_owner_key = theme_owner_quotas.catalog_owner_key), 0);
UPDATE theme_site_quota SET physical_bytes = physical_bytes + COALESCE((SELECT SUM(physical_bytes) FROM theme_draft_content_charges), 0) WHERE singleton = 1;
