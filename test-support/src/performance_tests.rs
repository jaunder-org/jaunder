//! Public library-seam tests for deterministic performance fixture population.

use std::collections::BTreeMap;

use performance::{CountOverrides, DatasetProfile, PersistedCursor, Workload, validate_manifest};
use rstest::*;
use rstest_reuse::*;
use storage::test_support::{Backend, backends};

use crate::performance::{PerformanceSeedStorage, seed_performance_fixture_with_audit};

#[apply(backends)]
#[tokio::test]
async fn small_fixture_resolves_a_valid_manifest_from_persisted_records(#[case] backend: Backend) {
    // crap:allow: the dual-backend rstest body is executed but macro expansion leaves test source without LCOV attribution
    let env = backend.setup().pristine().await;
    let output = tempfile::tempdir().expect("temporary manifest directory");
    let storage_root = tempfile::tempdir().expect("temporary Media storage root");
    let (manifest, audit, _) = seed_performance_fixture_with_audit(
        PerformanceSeedStorage {
            users: env.users(),
            posts: env.posts(),
            subscriptions: env.subscriptions(),
            audiences: env.audiences(),
            media: env.media(),
            write_scope: env.write_scope(),
        },
        DatasetProfile::Small,
        CountOverrides::default(),
        output.path(),
        storage_root.path(),
    )
    .await
    .expect("small fixture seeds through typed storage");
    let persisted: performance::DatasetManifest = serde_json::from_slice(
        &std::fs::read(output.path().join(performance::DATASET_MANIFEST_FILENAME))
            .expect("read emitted manifest"),
    )
    .expect("parse emitted manifest");
    validate_manifest(&persisted).expect("emitted manifest satisfies shared contract");

    validate_manifest(&manifest).expect("persisted manifest satisfies shared contract");
    assert_eq!(manifest.plan.posts, 100);
    assert_eq!(
        manifest
            .plan
            .lifecycle
            .iter()
            .take(4)
            .map(|item| item.count)
            .collect::<Vec<_>>(),
        [60, 15, 15, 10],
    );
    assert_eq!(
        manifest
            .plan
            .revision_distribution
            .iter()
            .map(|item| item.count)
            .collect::<Vec<_>>(),
        [70, 20, 10],
    );
    assert_eq!(
        manifest
            .plan
            .media_distribution
            .iter()
            .map(|item| item.count)
            .collect::<Vec<_>>(),
        [75, 20, 5],
    );
    assert_eq!(manifest.plan.revisions, 500);
    assert_eq!(
        manifest
            .plan
            .tag_distribution
            .iter()
            .map(|item| item.count)
            .collect::<Vec<_>>(),
        [25, 50, 25],
    );
    assert_eq!(
        manifest
            .plan
            .audience_distribution
            .iter()
            .map(|item| item.count)
            .collect::<Vec<_>>(),
        [50, 25, 25],
    );
    assert_eq!(
        manifest
            .plan
            .body_distribution
            .iter()
            .map(|item| item.count)
            .collect::<Vec<_>>(),
        [12, 11, 11, 11, 11, 11, 11, 11, 11],
    );
    assert_eq!(manifest.plan.lifecycle[4].count, 20);
    assert_eq!(manifest.plan.authors, 10);
    assert_eq!(manifest.plan.follows_per_author, 9);
    let expected_audit = BTreeMap::from([
        ("lifecycle.live".to_owned(), 60),
        ("lifecycle.draft".to_owned(), 15),
        ("lifecycle.scheduled".to_owned(), 15),
        ("lifecycle.deleted".to_owned(), 10),
        ("lifecycle.backdated_live".to_owned(), 20),
        ("revisions.one".to_owned(), 70),
        ("revisions.five".to_owned(), 20),
        ("revisions.thirty_three".to_owned(), 10),
        ("tags.none".to_owned(), 25),
        ("tags.two".to_owned(), 50),
        ("tags.eight".to_owned(), 25),
        ("audiences.public_only".to_owned(), 50),
        ("audiences.one_private".to_owned(), 25),
        ("audiences.five_private".to_owned(), 25),
        ("body.markdown_256".to_owned(), 12),
        ("body.markdown_4096".to_owned(), 11),
        ("body.markdown_65536".to_owned(), 11),
        ("body.html_256".to_owned(), 11),
        ("body.html_4096".to_owned(), 11),
        ("body.html_65536".to_owned(), 11),
        ("body.plain_text_256".to_owned(), 11),
        ("body.plain_text_4096".to_owned(), 11),
        ("body.plain_text_65536".to_owned(), 11),
        ("media.none".to_owned(), 75),
        ("media.one".to_owned(), 20),
        ("media.five".to_owned(), 5),
    ]);
    assert_eq!(audit.confirmed, expected_audit);
    assert_eq!(audit.persisted, audit.confirmed);
    assert!(manifest.subjects.history_post_id > 0);
    assert!(manifest.subjects.revision_id > 0);
    assert_eq!(manifest.subjects.browser_initial_rows.home, 50);
    assert_eq!(manifest.subjects.browser_initial_rows.app, 6);
    assert!(manifest.subjects.browser_initial_rows.global_history > 50);
    assert_eq!(manifest.subjects.browser_initial_rows.post_history, 32);
    assert_eq!(manifest.cursors.len(), 4);
    for cursor in &manifest.cursors {
        assert!(cursor.matching_result_count > 0);
        assert!(cursor.resolved_rank > 0);
        match cursor.workload {
            Workload::PublicTimeline | Workload::AuthenticatedTimeline => {
                assert!(matches!(cursor.cursor, PersistedCursor::Timeline(_)));
            }
            Workload::OwnerHistory | Workload::PostHistory => {
                assert!(matches!(cursor.cursor, PersistedCursor::History(_)));
            }
            _ => unreachable!("manifest contains only paginated storage workloads"),
        }
    }
    assert!(
        output
            .path()
            .join(performance::DATASET_MANIFEST_FILENAME)
            .is_file()
    );
    assert!(
        !output
            .path()
            .join(format!(".{}.tmp", performance::DATASET_MANIFEST_FILENAME))
            .exists(),
        "atomic publication leaves no temporary manifest",
    );
    let owner = env
        .users()
        .get_user_by_username(
            &manifest
                .subjects
                .username
                .parse()
                .expect("fixture username"),
        )
        .await
        .expect("typed author lookup succeeds")
        .expect("fixture author exists")
        .user_id;
    let history_post = common::ids::PostId::from(
        i64::try_from(manifest.subjects.history_post_id).expect("manifest post id fits storage id"),
    );
    let persisted_post = env
        .posts()
        .get_post_by_id(
            history_post,
            &common::visibility::ViewerIdentity::local(owner),
        )
        .await
        .expect("typed selected post query succeeds")
        .expect("selected post exists for its manifest owner");
    assert_eq!(persisted_post.user_id, owner);
    assert_eq!(persisted_post.body.len(), 65_536);
    let mut revision_count = 0;
    let mut cursor = None;
    loop {
        let page = env
            .posts()
            .list_post_revision_history(
                owner,
                history_post,
                cursor,
                common::pagination::PageSize::default(),
            )
            .await
            .expect("typed selected history query succeeds")
            .expect("selected post history exists");
        if page.revisions.is_empty() {
            break;
        }
        cursor = page
            .revisions
            .last()
            .map(|revision| storage::PostRevisionCursor {
                revision_id: revision.revision_id,
            });
        revision_count += page.revisions.len();
    }
    assert_eq!(revision_count + 1, 33);
    for index in 0..10 {
        let author = env
            .users()
            .get_user_by_username(
                &format!("perf-author-{index:04}")
                    .parse()
                    .expect("canonical fixture username"),
            )
            .await
            .expect("typed author lookup succeeds")
            .expect("canonical author exists")
            .user_id;
        let subscribers = env
            .subscriptions()
            .list_subscribers(author)
            .await
            .expect("typed subscription listing succeeds");
        assert_eq!(subscribers.len(), 9, "each author has nine ring peers");
        for other in 0..10 {
            if other == index {
                continue;
            }
            let viewer = env
                .users()
                .get_user_by_username(
                    &format!("perf-author-{other:04}")
                        .parse()
                        .expect("canonical fixture username"),
                )
                .await
                .expect("typed author lookup succeeds")
                .expect("canonical author exists")
                .user_id;
            assert!(
                env.subscriptions()
                    .is_subscriber(author, &common::visibility::ViewerIdentity::local(viewer))
                    .await
                    .expect("typed ring subscription lookup succeeds"),
                "every distinct author follows this ring peer",
            );
        }
        let audiences = env
            .audiences()
            .list_audiences(author)
            .await
            .expect("typed audience listing succeeds");
        assert_eq!(audiences.len(), 5);
        for audience in audiences {
            assert_eq!(
                env.audiences()
                    .list_members(author, audience.audience_id)
                    .await
                    .expect("typed audience membership listing succeeds")
                    .len(),
                9,
                "every fixture audience contains all ring peers",
            );
        }
    }
    let media = env
        .media()
        .list_media(
            owner,
            None,
            common::pagination::RowLimit::at_most(10),
            common::pagination::PageOffset::default(),
        )
        .await
        .expect("typed Media query succeeds");
    assert_eq!(media.len(), 5);
    for record in media {
        let content = storage_root.path().join("media").join(common::media::path(
            &record.source,
            &record.sha256,
            &record.filename,
        ));
        assert!(content.is_file(), "canonical Media bytes exist");
    }
}

#[apply(backends)]
#[tokio::test]
async fn overridden_fixture_records_and_validates_exact_requested_totals(#[case] backend: Backend) {
    let env = backend.setup().pristine().await;
    let output = tempfile::tempdir().expect("temporary manifest directory");
    let storage_root = tempfile::tempdir().expect("temporary Media storage root");
    let overrides = CountOverrides {
        posts: Some(120),
        authors: Some(12),
        revisions: Some(777),
    };
    let (manifest, audit, _) = seed_performance_fixture_with_audit(
        PerformanceSeedStorage {
            users: env.users(),
            posts: env.posts(),
            subscriptions: env.subscriptions(),
            audiences: env.audiences(),
            media: env.media(),
            write_scope: env.write_scope(),
        },
        DatasetProfile::Small,
        overrides,
        output.path(),
        storage_root.path(),
    )
    .await
    .expect("overridden fixture seeds through typed storage");

    assert_eq!(
        (
            manifest.plan.posts,
            manifest.plan.authors,
            manifest.plan.revisions
        ),
        (120, 12, 777)
    );
    assert_eq!(audit.persisted, audit.confirmed);
    validate_manifest(&manifest).expect("overridden manifest satisfies shared contract");
}

mod contract_coverage {
    use performance::*;

    fn manifest() -> DatasetManifest {
        let plan = canonical_plan(DatasetProfile::Small);
        DatasetManifest {
            schema_version: DATASET_SCHEMA_VERSION,
            plan,
            subjects: WorkloadSubjects {
                username: "performance".into(),
                history_post_id: 1,
                revision_id: 1,
                browser_initial_rows: BrowserInitialRows {
                    home: 100,
                    app: 100,
                    global_history: 100,
                    post_history: 100,
                },
            },
            cursors: [
                Workload::PublicTimeline,
                Workload::AuthenticatedTimeline,
                Workload::OwnerHistory,
                Workload::PostHistory,
            ]
            .into_iter()
            .map(|workload| Cursor {
                workload,
                target_percent: 80,
                matching_result_count: 100,
                resolved_rank: 80,
                cursor: if matches!(
                    workload,
                    Workload::PublicTimeline | Workload::AuthenticatedTimeline
                ) {
                    PersistedCursor::Timeline(TimelineCursor {
                        created_at_us: 1,
                        post_id: 1,
                    })
                } else {
                    PersistedCursor::History(HistoryCursor { revision_id: 1 })
                },
            })
            .collect(),
        }
    }

    fn item(
        workload: Workload,
        frame: MeasurementFrame,
        position: MeasurementPosition,
    ) -> WorkloadResult {
        // crap:allow: this integration fixture builder mirrors the versioned compatibility key and is exercised but test source has no LCOV attribution
        let plan = canonical_plan(DatasetProfile::Small);
        let browser = matches!(
            workload,
            Workload::Home
                | Workload::App
                | Workload::GlobalHistory
                | Workload::BrowserPostHistory
                | Workload::BrowserRevisionDetail
        )
        .then_some(Browser::Chromium);
        let count = if browser.is_some() {
            20
        } else {
            match frame {
                MeasurementFrame::Cold => 1,
                MeasurementFrame::Warm => 30,
            }
        };
        let samples = (1..=u64::from(count))
            .map(|duration_us| RawSample { duration_us })
            .collect::<Vec<_>>();
        let (page_size, cursor_target_percent, cursor_resolved_rank) = match position {
            MeasurementPosition::Initial => (Some(50), None, None),
            MeasurementPosition::Deep if workload == Workload::GlobalHistory => {
                (Some(50), None, None)
            }
            MeasurementPosition::Deep => (Some(50), Some(80), Some(80)),
            MeasurementPosition::Point => (None, None, None),
        };
        WorkloadResult {
            key: CompatibilityKey {
                result_schema_version: RESULT_SCHEMA_VERSION,
                generator: plan.generator,
                profile: plan.profile,
                workload,
                backend: Backend::Sqlite,
                browser,
                build_mode: BuildMode::Release,
                measurement_frame: frame,
                measurement_position: position,
                sample_count: count,
                page_size,
                cursor_target_percent,
                cursor_resolved_rank,
                nix_system: "x86_64-linux".into(),
                stable_derivation_identities: vec![NamedDerivationIdentity {
                    name: "producer".into(),
                    identity: "stable".into(),
                }],
                runner_image: "image".into(),
                runner_architecture: "x86_64".into(),
                cpu_model: "cpu".into(),
                database_version: "database".into(),
                browser_version: browser.map(|_| "browser".into()),
            },
            summary: summarize(&samples).expect("valid samples"),
            samples,
            rows_returned: 50,
        }
    }

    fn storage_workloads() -> Vec<WorkloadResult> {
        let mut workloads = Vec::new();
        for workload in [
            Workload::PublicTimeline,
            Workload::AuthenticatedTimeline,
            Workload::OwnerHistory,
            Workload::PostHistory,
        ] {
            for frame in [MeasurementFrame::Cold, MeasurementFrame::Warm] {
                for position in [MeasurementPosition::Initial, MeasurementPosition::Deep] {
                    workloads.push(item(workload, frame, position));
                }
            }
        }
        for frame in [MeasurementFrame::Cold, MeasurementFrame::Warm] {
            workloads.push(item(
                Workload::RevisionDetail,
                frame,
                MeasurementPosition::Point,
            ));
        }
        workloads
    }

    fn browser_workloads() -> Vec<WorkloadResult> {
        [
            (Workload::Home, MeasurementPosition::Initial),
            (Workload::App, MeasurementPosition::Initial),
            (Workload::GlobalHistory, MeasurementPosition::Initial),
            (Workload::GlobalHistory, MeasurementPosition::Deep),
            (Workload::BrowserPostHistory, MeasurementPosition::Initial),
            (Workload::BrowserRevisionDetail, MeasurementPosition::Point),
        ]
        .into_iter()
        .map(|(workload, position)| item(workload, MeasurementFrame::Cold, position))
        .collect()
    }

    fn storage_envelope(workloads: Vec<WorkloadResult>) -> FragmentEnvelope {
        FragmentEnvelope {
            schema_version: RESULT_SCHEMA_VERSION,
            manifest: manifest(),
            fragment: Fragment::Storage(StorageFragment {
                setup: SetupDuration {
                    producer: Producer::Storage,
                    backend: Backend::Sqlite,
                    provisioning_us: 1,
                    seeding_us: 1,
                },
                workloads,
            }),
        }
    }

    fn browser_envelope(workloads: Vec<WorkloadResult>) -> FragmentEnvelope {
        FragmentEnvelope {
            schema_version: RESULT_SCHEMA_VERSION,
            manifest: manifest(),
            fragment: Fragment::Browser(BrowserFragment {
                setup: SetupDuration {
                    producer: Producer::Browser,
                    backend: Backend::Sqlite,
                    provisioning_us: 1,
                    seeding_us: 1,
                },
                diagnostics: BrowserDiagnostics {
                    navigation_artifacts: (0..workloads.len() * 20)
                        .map(|index| format!("navigation-{index}.json"))
                        .collect(),
                    trace_artifacts: (0..workloads.len() * 20)
                        .map(|index| format!("trace-{index}.zip"))
                        .collect(),
                    otel_trace_artifact: "otel-traces.jsonl".into(),
                },
                workloads,
            }),
        }
    }

    #[test]
    fn filenames_and_noncanonical_plans_cover_the_public_contract() {
        assert_eq!(
            storage_fragment_filename(Backend::Sqlite),
            "storage-sqlite-v1.json"
        );
        assert_eq!(
            storage_fragment_filename(Backend::Postgres),
            "storage-postgres-v1.json"
        );
        assert_eq!(
            browser_fragment_filename(Backend::Sqlite, Browser::Chromium),
            "browser-sqlite-chromium-v1.json"
        );
        assert_eq!(
            browser_fragment_filename(Backend::Postgres, Browser::Firefox),
            "browser-postgres-firefox-v1.json"
        );
        assert_eq!(
            plan(
                DatasetProfile::Small,
                CountOverrides {
                    posts: Some(0),
                    ..CountOverrides::default()
                }
            ),
            Err(PlanError::Posts)
        );
        assert_eq!(
            plan(
                DatasetProfile::Small,
                CountOverrides {
                    authors: Some(0),
                    ..CountOverrides::default()
                }
            ),
            Err(PlanError::Authors)
        );
        let remainder = plan(
            DatasetProfile::Small,
            CountOverrides {
                posts: Some(121),
                authors: Some(11),
                revisions: Some(777),
            },
        )
        .expect("valid uneven weighted allocation");
        assert_eq!(
            remainder
                .lifecycle
                .iter()
                .map(|item| item.count)
                .sum::<u64>(),
            145
        );
    }

    #[test]
    fn manifest_validation_rejects_each_cursor_integrity_boundary() {
        let mut invalid = manifest();
        invalid.schema_version += 1;
        assert_eq!(validate_manifest(&invalid), Err(ManifestError::Schema));

        let mut invalid = manifest();
        invalid.cursors[0].workload = Workload::AuthenticatedTimeline;
        assert_eq!(validate_manifest(&invalid), Err(ManifestError::Cursor));

        let mut invalid = manifest();
        invalid.cursors[0].target_percent = 79;
        assert_eq!(validate_manifest(&invalid), Err(ManifestError::Cursor));

        let mut invalid = manifest();
        invalid.subjects.browser_initial_rows.home -= 1;
        assert_eq!(validate_manifest(&invalid), Err(ManifestError::Cursor));
    }

    #[test]
    fn validates_complete_storage_and_browser_fragments() {
        assert_eq!(
            validate_fragment(&storage_envelope(storage_workloads())),
            Ok(())
        );
        assert_eq!(
            validate_fragment(&browser_envelope(browser_workloads())),
            Ok(())
        );
    }

    #[test]
    fn validates_a_complete_fragment_with_custom_counts() {
        let mut envelope = storage_envelope(storage_workloads());
        envelope.manifest.plan = plan(
            DatasetProfile::Small,
            CountOverrides {
                posts: Some(120),
                authors: Some(12),
                revisions: Some(777),
            },
        )
        .expect("custom plan");
        assert_eq!(validate_fragment(&envelope), Ok(()));
    }

    #[test]
    fn rejects_missing_and_duplicate_workloads() {
        let mut workloads = storage_workloads();
        workloads.pop();
        assert!(matches!(
            validate_fragment(&storage_envelope(workloads)),
            Err(FragmentError::Missing(_))
        ));

        let mut workloads = storage_workloads();
        workloads.push(workloads[0].clone());
        assert!(matches!(
            validate_fragment(&storage_envelope(workloads)),
            Err(FragmentError::Duplicate(_))
        ));
    }

    #[test]
    fn rejects_an_unsupported_fragment_schema() {
        let mut envelope = storage_envelope(storage_workloads());
        envelope.schema_version += 1;
        assert_eq!(
            validate_fragment(&envelope),
            Err(FragmentError::UnsupportedSchema)
        );
    }

    #[test]
    fn rejects_invalid_manifest_with_its_specific_error() {
        for username in ["", " "] {
            let mut fragment = storage_envelope(storage_workloads());
            fragment.manifest.subjects.username = username.into();
            assert_eq!(
                validate_fragment(&fragment),
                Err(FragmentError::InvalidManifest(ManifestError::Cursor))
            );
        }
    }

    #[test]
    fn rejects_setup_identity_and_backend_mismatches() {
        let mut fragment = storage_envelope(storage_workloads());
        let Fragment::Storage(storage) = &mut fragment.fragment else {
            unreachable!("constructed storage fragment")
        };
        storage.setup.producer = Producer::Browser;
        assert_eq!(
            validate_fragment(&fragment),
            Err(FragmentError::SetupProducerMismatch)
        );

        let mut fragment = storage_envelope(storage_workloads());
        let Fragment::Storage(storage) = &mut fragment.fragment else {
            unreachable!("constructed storage fragment")
        };
        storage.setup.backend = Backend::Postgres;
        assert_eq!(
            validate_fragment(&fragment),
            Err(FragmentError::SetupBackendMismatch)
        );
    }

    #[test]
    fn rejects_noncanonical_samples_summaries_and_builds() {
        let mut workloads = storage_workloads();
        workloads[0].samples.push(RawSample { duration_us: 2 });
        let mut summary = storage_workloads();
        summary[0].summary.mean_us += 1;
        let mut debug = storage_workloads();
        debug[0].key.build_mode = BuildMode::Debug;
        for workloads in [workloads, summary, debug] {
            assert_eq!(
                validate_fragment(&storage_envelope(workloads)),
                Err(FragmentError::IncompatibleWorkload)
            );
        }
    }

    #[test]
    fn rejects_cross_producer_and_bad_position_workloads() {
        let mut cross = storage_workloads();
        cross[0].key.workload = Workload::Home;
        let mut page = storage_workloads();
        page[0].key.page_size = None;
        let mut point = storage_workloads();
        point[0].key.measurement_position = MeasurementPosition::Point;
        point[0].key.page_size = None;
        for workloads in [cross, page, point] {
            assert_eq!(
                validate_fragment(&storage_envelope(workloads)),
                Err(FragmentError::IncompatibleWorkload)
            );
        }

        let mut wrong_producer = browser_workloads();
        wrong_producer[0].key.workload = Workload::PublicTimeline;
        assert_eq!(
            validate_fragment(&browser_envelope(wrong_producer)),
            Err(FragmentError::IncompatibleWorkload)
        );
    }

    #[test]
    fn rejects_a_browser_fragment_without_a_browser_identity() {
        let mut workloads = browser_workloads();
        workloads[0].key.browser = None;
        assert_eq!(
            validate_fragment(&browser_envelope(workloads)),
            Err(FragmentError::IncompatibleWorkload)
        );
    }

    #[test]
    fn rejects_a_workload_for_another_browser() {
        let mut workloads = browser_workloads();
        workloads[1].key.browser = Some(Browser::Firefox);
        assert!(matches!(
            validate_fragment(&browser_envelope(workloads)),
            Err(FragmentError::Unselected(_))
        ));
    }

    #[test]
    fn rejects_browser_warm_measurements_and_wrong_deep_cursor_rank() {
        let mut browser_warm = item(
            Workload::Home,
            MeasurementFrame::Warm,
            MeasurementPosition::Initial,
        );
        browser_warm.key.sample_count = 20;
        browser_warm.samples = vec![RawSample { duration_us: 1 }; 20];
        browser_warm.summary = summarize(&browser_warm.samples).expect("valid samples");
        assert_eq!(
            validate_fragment(&browser_envelope(vec![browser_warm])),
            Err(FragmentError::IncompatibleWorkload)
        );

        let mut workloads = storage_workloads();
        workloads[1].key.cursor_resolved_rank = Some(79);
        assert_eq!(
            validate_fragment(&storage_envelope(workloads)),
            Err(FragmentError::IncompatibleWorkload)
        );
    }

    #[test]
    fn browser_deep_positions_resolve_their_storage_cursors_but_are_not_owed() {
        for workload in [Workload::Home, Workload::App, Workload::BrowserPostHistory] {
            let mut workloads = browser_workloads();
            workloads.insert(
                0,
                item(workload, MeasurementFrame::Cold, MeasurementPosition::Deep),
            );
            assert!(matches!(
                validate_fragment(&browser_envelope(workloads)),
                Err(FragmentError::Unselected(_))
            ));
        }
    }

    #[test]
    fn rejects_blank_or_missing_browser_diagnostics_and_stable_derivations() {
        let mut workloads = storage_workloads();
        workloads[0].key.stable_derivation_identities[0]
            .identity
            .clear();
        assert_eq!(
            validate_fragment(&storage_envelope(workloads)),
            Err(FragmentError::IncompatibleWorkload)
        );

        let mut duplicate = browser_envelope(browser_workloads());
        let Fragment::Browser(fragment) = &mut duplicate.fragment else {
            unreachable!("constructed browser fragment")
        };
        fragment.diagnostics.trace_artifacts[1] = fragment.diagnostics.trace_artifacts[0].clone();
        assert_eq!(
            validate_fragment(&duplicate),
            Err(FragmentError::InvalidDiagnostics)
        );

        for diagnostics in [
            BrowserDiagnostics {
                navigation_artifacts: vec![" ".into()],
                trace_artifacts: vec!["trace.json".into()],
                otel_trace_artifact: "otel-traces.jsonl".into(),
            },
            BrowserDiagnostics {
                navigation_artifacts: vec![],
                trace_artifacts: vec!["trace.json".into()],
                otel_trace_artifact: "otel-traces.jsonl".into(),
            },
            BrowserDiagnostics {
                navigation_artifacts: vec![],
                trace_artifacts: vec![],
                otel_trace_artifact: " ".into(),
            },
        ] {
            let fragment = FragmentEnvelope {
                schema_version: RESULT_SCHEMA_VERSION,
                manifest: manifest(),
                fragment: Fragment::Browser(BrowserFragment {
                    setup: SetupDuration {
                        producer: Producer::Browser,
                        backend: Backend::Sqlite,
                        provisioning_us: 1,
                        seeding_us: 1,
                    },
                    diagnostics,
                    workloads: vec![],
                }),
            };
            assert_eq!(
                validate_fragment(&fragment),
                Err(FragmentError::InvalidDiagnostics)
            );
        }
    }

    #[test]
    fn rejects_duplicate_or_unsorted_stable_derivation_names() {
        for name in ["producer", "alpha"] {
            let mut workloads = storage_workloads();
            workloads[0]
                .key
                .stable_derivation_identities
                .push(NamedDerivationIdentity {
                    name: name.into(),
                    identity: "other".into(),
                });
            assert_eq!(
                validate_fragment(&storage_envelope(workloads)),
                Err(FragmentError::IncompatibleWorkload)
            );
        }
    }
}
