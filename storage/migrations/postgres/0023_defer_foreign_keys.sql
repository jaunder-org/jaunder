-- Make every foreign key DEFERRABLE INITIALLY IMMEDIATE so a restore can
-- SET CONSTRAINTS ALL DEFERRED and bulk-load rows in any order, with integrity
-- verified once at COMMIT. INITIALLY IMMEDIATE keeps normal operation unchanged:
-- checks stay immediate except inside a transaction that explicitly defers.
-- The DO block discovers constraint names from the catalog rather than hard-coding
-- Postgres's generated `<table>_<column>_fkey` names.
DO $$
DECLARE r RECORD;
BEGIN
    FOR r IN
        SELECT conrelid::regclass AS tbl, conname
        FROM pg_constraint
        WHERE contype = 'f' AND connamespace = 'public'::regnamespace
    LOOP
        EXECUTE format(
            'ALTER TABLE %s ALTER CONSTRAINT %I DEFERRABLE INITIALLY IMMEDIATE',
            r.tbl, r.conname
        );
    END LOOP;
END $$;
