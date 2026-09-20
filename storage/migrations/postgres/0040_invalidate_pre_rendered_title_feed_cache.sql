-- Feed title serialization now consumes persisted Rendered Titles. Existing cache
-- bodies predate that representation and must not be served after upgrade.
DELETE FROM feed_cache;
