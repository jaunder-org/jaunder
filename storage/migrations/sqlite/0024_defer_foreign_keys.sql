-- No-op: parity placeholder so SQLite and Postgres share schema_version 24 (the
-- backup manifest's schema_version is MAX(version), and the cross-backend interop
-- tests compare it). SQLite restore disables FK enforcement per-connection
-- (PRAGMA foreign_keys = OFF) and validates once via foreign_key_check, so it
-- needs no schema change; altering SQLite FK deferrability would require a full
-- table rebuild for no benefit.
SELECT 1;
