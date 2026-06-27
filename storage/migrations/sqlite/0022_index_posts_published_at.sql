-- Scheduled publishing (#70): public reads now gate on
-- `published_at <= now`, and the feed worker's go-live window/catch-up scans
-- range over `published_at`. A standalone partial index supports those scans;
-- the existing composite slug index does not (its leading column is user_id).
CREATE INDEX IF NOT EXISTS idx_posts_published_at
    ON posts (published_at)
    WHERE deleted_at IS NULL;
