CREATE TABLE publisher_state (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    generation INTEGER NOT NULL CHECK (generation >= 0)
);

INSERT INTO publisher_state (id, generation) VALUES (1, 0);
