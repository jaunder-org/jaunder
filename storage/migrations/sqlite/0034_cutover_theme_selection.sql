-- The typed theme aggregate owns public selection after Task 5.
DELETE FROM site_config WHERE key = 'site.theme';
DELETE FROM user_config WHERE key = 'user.theme';
