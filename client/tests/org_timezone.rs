#![cfg(target_arch = "wasm32")]

use common::{
    org::{OrgOperation, OrgStructuredMetadata, Presence, PublicationState, normalize_org},
    time::UtcInstant,
};
use jiff::tz::TimeZone;
use wasm_bindgen_test::wasm_bindgen_test;

wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

const NAMED_ZONE_ORG_FIXTURE: &str = "\
#+DATE: [2026-11-01 Sun 01:30]
#+PROPERTY: JAUNDER_DATE_TZ America/New_York
#+PROPERTY: JAUNDER_STATUS scheduled
Body";

#[wasm_bindgen_test]
fn named_zone_org_fixture_resolves_in_the_browser() {
    let timezone = TimeZone::get("America/New_York").expect("bundled named zone");
    let normalized = normalize_org(
        NAMED_ZONE_ORG_FIXTURE,
        OrgStructuredMetadata::default(),
        OrgOperation::Create,
        "2026-08-26T12:00:00Z"
            .parse::<UtcInstant>()
            .expect("fixed clock"),
    )
    .expect("valid Org metadata");

    assert_eq!(timezone.iana_name(), Some("America/New_York"));
    assert_eq!(
        normalized.metadata.lifecycle,
        Presence::Present(PublicationState::Scheduled(
            "2026-11-01T05:30:00Z"
                .parse()
                .expect("expected UTC instant")
        ))
    );
}
