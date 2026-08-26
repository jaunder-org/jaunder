use std::{
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

use async_trait::async_trait;
use common::{ids::PostId, media::parse_media_url, tagged_url::BaseUrl};
use storage::PersistedMediaReference;
use tokio::sync::Mutex;
use url::Url;

use super::resolver::MAX_FOREIGN_EVIDENCE;
use super::*;

#[derive(Default)]
struct FakeTransport {
    responses: Mutex<Vec<HeadResponse>>,
    heads: AtomicUsize,
    in_flight_heads: AtomicUsize,
    max_in_flight_heads: AtomicUsize,
    head_delay: Option<Duration>,
    seen_targets: Mutex<Vec<Url>>,
}

impl FakeTransport {
    fn with_responses(responses: Vec<HeadResponse>) -> Self {
        Self {
            responses: Mutex::new(responses),
            ..Self::default()
        }
    }

    fn with_delayed_responses(responses: Vec<HeadResponse>, delay: Duration) -> Self {
        Self {
            head_delay: Some(delay),
            ..Self::with_responses(responses)
        }
    }
}

#[async_trait]
impl HeadTransport for FakeTransport {
    async fn head(&self, target: &Url) -> Result<HeadResponse, HeadTransportError> {
        self.heads.fetch_add(1, Ordering::Relaxed);
        let in_flight = self.in_flight_heads.fetch_add(1, Ordering::Relaxed) + 1;
        self.max_in_flight_heads
            .fetch_max(in_flight, Ordering::Relaxed);
        if let Some(delay) = self.head_delay {
            tokio::time::sleep(delay).await;
        }
        self.in_flight_heads.fetch_sub(1, Ordering::Relaxed);
        self.seen_targets.lock().await.push(target.clone());
        Ok(self.responses.lock().await.remove(0))
    }
}

struct FailingTransport;

#[async_trait]
impl HeadTransport for FailingTransport {
    async fn head(&self, _: &Url) -> Result<HeadResponse, HeadTransportError> {
        reqwest::Client::new()
            .head("http://127.0.0.1:9/")
            .send()
            .await
            .map_err(HeadTransportError::Request)?;
        unreachable!("the closed local port must fail")
    }
}

fn reference(post: i64, form: &str) -> PersistedMediaReference {
    let parsed = parse_media_url(form).expect("valid media reference form");
    PersistedMediaReference::new(
        PostId::from(post),
        parsed.media().clone(),
        parsed.kind(),
        parsed.reference_form().clone(),
    )
}

fn instance_id() -> storage::InstanceId {
    "123e4567-e89b-12d3-a456-426614174000".parse().unwrap()
}

fn foreign() -> HeadResponse {
    HeadResponse::new(Vec::new())
}

fn media_path() -> &'static str {
    "/media/upload/00/00/0000000000000000000000000000000000000000000000000000000000000000/photo.jpg"
}

#[test]
fn live_resolver_default_constructs_the_reqwest_transport() {
    let _: LiveMediaReferenceOwnershipResolver = LiveMediaReferenceOwnershipResolver::default();
}

#[tokio::test]
async fn foreign_response_creates_evidence_for_every_exactly_deduplicated_target_row() {
    let transport = FakeTransport::with_responses(vec![foreign()]);
    let resolver = LiveMediaReferenceOwnershipResolver::with_transport(transport);
    let target = format!("https://example.test{}", media_path());
    let references = vec![reference(1, &target), reference(2, &target)];

    let evidence = resolver.resolve(&references, &instance_id(), None).await;

    assert_eq!(evidence.references().len(), 2);
    assert_eq!(resolver.transport().heads.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn scheme_relative_reference_inherits_only_the_configured_scheme() {
    let transport = FakeTransport::with_responses(vec![foreign()]);
    let resolver = LiveMediaReferenceOwnershipResolver::with_transport(transport);
    let reference = reference(
        1,
        &format!("//media.example:8443{}?kept=query", media_path()),
    );
    let base: BaseUrl = "https://configured.example/base".parse().unwrap();

    assert!(
        resolver
            .resolve(std::slice::from_ref(&reference), &instance_id(), None)
            .await
            .references()
            .is_empty()
    );
    assert_eq!(
        resolver
            .resolve(
                std::slice::from_ref(&reference),
                &instance_id(),
                Some(&base)
            )
            .await
            .references()
            .len(),
        1
    );
    assert_eq!(
        resolver.transport().seen_targets.lock().await[0].as_str(),
        format!("https://media.example:8443{}?kept=query", media_path())
    );
}

#[tokio::test]
async fn exactly_one_canonical_matching_uuid_is_owned() {
    let expected = instance_id().to_string().into_bytes();
    let resolver =
        LiveMediaReferenceOwnershipResolver::with_transport(FakeTransport::with_responses(vec![
            HeadResponse::new(vec![expected]),
        ]));

    let evidence = resolver
        .resolve(
            &[reference(
                1,
                &format!("https://example.test{}", media_path()),
            )],
            &instance_id(),
            None,
        )
        .await;

    assert!(evidence.references().is_empty());
}

#[tokio::test]
async fn absent_or_different_uuid_is_foreign() {
    let other = b"123e4567-e89b-12d3-a456-426614174001".to_vec();
    let resolver =
        LiveMediaReferenceOwnershipResolver::with_transport(FakeTransport::with_responses(vec![
            foreign(),
            HeadResponse::new(vec![other]),
        ]));
    let reference = reference(1, &format!("https://example.test{}", media_path()));

    assert_eq!(
        resolver
            .resolve(std::slice::from_ref(&reference), &instance_id(), None)
            .await
            .references()
            .len(),
        1
    );
    assert_eq!(
        resolver
            .resolve(std::slice::from_ref(&reference), &instance_id(), None)
            .await
            .references()
            .len(),
        1
    );
}

#[tokio::test]
async fn malformed_or_ambiguous_uuid_is_unknown() {
    let responses = vec![
        HeadResponse::new(vec![b"not-a-uuid".to_vec()]),
        HeadResponse::new(vec![
            b"123e4567-e89b-12d3-a456-426614174000".to_vec(),
            b"123e4567-e89b-12d3-a456-426614174001".to_vec(),
        ]),
        HeadResponse::new(vec![b"123e4567-e89b-12d3-a456-426614174000, x".to_vec()]),
    ];
    let resolver = LiveMediaReferenceOwnershipResolver::with_transport(
        FakeTransport::with_responses(responses),
    );
    let reference = reference(1, &format!("https://example.test{}", media_path()));

    for _ in 0..3 {
        assert!(
            resolver
                .resolve(std::slice::from_ref(&reference), &instance_id(), None)
                .await
                .references()
                .is_empty()
        );
    }
}

#[tokio::test]
async fn request_failure_leaves_reference_unknown() {
    let resolver = LiveMediaReferenceOwnershipResolver::with_transport(FailingTransport);
    let reference = reference(1, &format!("https://example.test{}", media_path()));

    assert!(
        resolver
            .resolve(&[reference], &instance_id(), None)
            .await
            .references()
            .is_empty()
    );
}

#[tokio::test]
async fn probes_no_more_than_eight_targets_concurrently() {
    let transport =
        FakeTransport::with_delayed_responses(vec![foreign(); 9], Duration::from_millis(20));
    let resolver = LiveMediaReferenceOwnershipResolver::with_transport(transport);
    let references: Vec<_> = (0_i64..9)
        .map(|post| {
            reference(
                post,
                &format!("https://{post}.example.test{}", media_path()),
            )
        })
        .collect();

    let evidence = resolver.resolve(&references, &instance_id(), None).await;

    assert_eq!(evidence.references().len(), 9);
    assert_eq!(
        resolver
            .transport()
            .max_in_flight_heads
            .load(Ordering::Relaxed),
        8
    );
}

#[tokio::test(start_paused = true)]
async fn operation_timeout_leaves_unfinished_probe_waves_unknown() {
    let resolver = std::sync::Arc::new(LiveMediaReferenceOwnershipResolver::with_transport(
        FakeTransport::with_delayed_responses(
            vec![foreign(); MAX_FOREIGN_EVIDENCE],
            Duration::from_secs(1),
        ),
    ));
    let references: Vec<_> = (0_i64..i64::try_from(MAX_FOREIGN_EVIDENCE).unwrap())
        .map(|post| {
            reference(
                post,
                &format!("https://{post}.example.test{}", media_path()),
            )
        })
        .collect();
    let final_reference = references.last().unwrap().clone();
    let task = {
        let resolver = std::sync::Arc::clone(&resolver);
        tokio::spawn(async move { resolver.resolve(&references, &instance_id(), None).await })
    };
    tokio::task::yield_now().await;

    tokio::time::advance(Duration::from_secs(10)).await;
    let evidence = task.await.unwrap();

    assert!(evidence.references().len() < MAX_FOREIGN_EVIDENCE);
    assert!(!evidence.proves_foreign(&final_reference));
}

#[tokio::test]
async fn reqwest_transport_issues_a_real_local_head_request() {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::mpsc,
    };

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (request_tx, request_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0; 4096];
        let length = stream.read(&mut request).unwrap();
        request_tx
            .send(String::from_utf8(request[..length].to_vec()).unwrap())
            .unwrap();
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\n\
                  x-jaunder-instance: 00000000-0000-0000-0000-000000000000\r\n\
                  content-length: 0\r\n\r\n",
            )
            .unwrap();
    });
    let resolver = LiveMediaReferenceOwnershipResolver::default();
    let target = format!("http://127.0.0.1:{}{}", address.port(), media_path());

    let evidence = resolver
        .resolve(&[reference(1, &target)], &instance_id(), None)
        .await;

    assert_eq!(evidence.references().len(), 1);
    assert!(
        request_rx
            .recv()
            .unwrap()
            .starts_with(&format!("HEAD {} HTTP/1.1\r\n", media_path()))
    );
}
