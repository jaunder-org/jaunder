-- Enforcement belongs to the offline handler, after earlier queued writers.
INSERT INTO pending_code_migrations (operation) VALUES ('normalize_comparable_instants_v1');
