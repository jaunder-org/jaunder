CREATE TABLE channels (
    channel_id BIGSERIAL PRIMARY KEY,
    name       TEXT NOT NULL UNIQUE
);
INSERT INTO channels (name) VALUES ('local');

CREATE TABLE subscription_statuses (
    status_id BIGSERIAL PRIMARY KEY,
    name      TEXT NOT NULL UNIQUE
);
INSERT INTO subscription_statuses (name) VALUES ('active');

CREATE TABLE target_kinds (
    kind_id BIGSERIAL PRIMARY KEY,
    name    TEXT NOT NULL UNIQUE
);
INSERT INTO target_kinds (name) VALUES ('public'), ('subscribers'), ('named');
