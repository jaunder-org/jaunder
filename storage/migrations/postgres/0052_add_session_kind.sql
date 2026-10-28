-- Legacy credentials have no reliable purpose discriminator. Never infer from labels.
ALTER TABLE sessions ADD COLUMN kind TEXT NOT NULL DEFAULT 'unknown'
    CHECK (kind IN ('browser', 'machine', 'unknown'));
