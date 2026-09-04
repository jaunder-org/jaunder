CREATE TABLE publisher_state (
    id BIGINT PRIMARY KEY CHECK (id = 1),
    generation BIGINT NOT NULL CHECK (generation >= 0)
);

INSERT INTO publisher_state (id, generation) VALUES (1, 0);
