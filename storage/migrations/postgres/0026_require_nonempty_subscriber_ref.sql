ALTER TABLE subscriptions
    ADD CONSTRAINT subscriptions_subscriber_ref_nonempty
    CHECK (subscriber_ref <> '');
