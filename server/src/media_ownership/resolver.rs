use std::{collections::BTreeMap, str::FromStr, sync::Arc, time::Duration};

use async_trait::async_trait;
use common::media::MediaReferenceKind;
use common::tagged_url::BaseUrl;
use futures_util::{StreamExt, stream};
use host::error;
use storage::{
    ForeignEvidenceSink, InstanceId, MediaReferenceEvidence, MediaReferenceOwnershipResolver,
    PersistedMediaReference,
};
use tokio::time::timeout;
use url::Url;

use super::{HeadResponse, HeadTransport, HeadTransportError, ReqwestHeadTransport};

const MAX_CONCURRENT_PROBES: usize = 8;
pub(super) const MAX_FOREIGN_EVIDENCE: usize = 128;
const OPERATION_TIMEOUT: Duration = Duration::from_secs(10);
/// A live resolver parameterized over its narrow DNS/HEAD transport seam.
pub struct LiveMediaReferenceOwnershipResolver<T = ReqwestHeadTransport> {
    transport: Arc<T>,
}

impl LiveMediaReferenceOwnershipResolver<ReqwestHeadTransport> {
    #[must_use]
    pub fn new() -> Self {
        Self::with_transport(ReqwestHeadTransport::default())
    }
}

impl Default for LiveMediaReferenceOwnershipResolver<ReqwestHeadTransport> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> LiveMediaReferenceOwnershipResolver<T> {
    #[must_use]
    pub fn with_transport(transport: T) -> Self {
        Self {
            transport: Arc::new(transport),
        }
    }

    #[cfg(test)]
    pub(super) async fn resolve(
        &self,
        references: &[PersistedMediaReference],
        instance_id: &InstanceId,
        base_url: Option<&BaseUrl>,
    ) -> MediaReferenceEvidence
    where
        T: HeadTransport + 'static,
    {
        storage::resolve_media_reference_ownership(self, references, instance_id, base_url).await
    }

    #[cfg(test)]
    pub(super) fn transport(&self) -> &T {
        self.transport.as_ref()
    }
}

#[async_trait]
impl<T: HeadTransport + 'static> MediaReferenceOwnershipResolver
    for LiveMediaReferenceOwnershipResolver<T>
{
    async fn resolve(
        &self,
        references: &[PersistedMediaReference],
        instance_id: &InstanceId,
        base_url: Option<&BaseUrl>,
        mut foreign: ForeignEvidenceSink,
    ) -> MediaReferenceEvidence {
        let operation = async {
            let targets = group_targets(references, base_url);
            let probes = stream::iter(targets.into_values().map(|(target, references)| {
                let transport = Arc::clone(&self.transport);
                let instance_id = instance_id.clone();
                async move {
                    let result = probe(transport.as_ref(), target, &instance_id).await;
                    (references, result)
                }
            }))
            .buffer_unordered(MAX_CONCURRENT_PROBES);
            tokio::pin!(probes);

            while let Some((references, result)) = probes.next().await {
                match result {
                    Ok(Ownership::Foreign) => {
                        for reference in references {
                            foreign.prove_foreign(reference);
                        }
                    }
                    Ok(Ownership::Owned | Ownership::Unknown) => {}
                    Err(error) => report_probe_error(&error),
                }
            }
        };
        if let Err(error) = timeout(OPERATION_TIMEOUT, operation).await {
            // A timed-out operation intentionally leaves unfinished rows live.
            error::report_swallowed(
                host::error::ErrorKind::Internal,
                host::error::ErrorClass::Transient,
                "server.media_ownership.operation_timeout",
                host::error::SwallowedSource::Error(&error),
            );
        }
        foreign.finish()
    }
}

fn group_targets(
    references: &[PersistedMediaReference],
    base_url: Option<&BaseUrl>,
) -> BTreeMap<String, (Url, Vec<PersistedMediaReference>)> {
    let mut targets = BTreeMap::new();
    for reference in references.iter().take(MAX_FOREIGN_EVIDENCE) {
        let Some(target) = target_for(reference, base_url) else {
            continue;
        };
        let key = target.to_string();
        targets
            .entry(key)
            .or_insert_with(|| (target, Vec::new()))
            .1
            .push(reference.clone());
    }
    targets
}

fn target_for(reference: &PersistedMediaReference, base_url: Option<&BaseUrl>) -> Option<Url> {
    let form = reference.reference_form();
    match reference.kind() {
        MediaReferenceKind::Local => None,
        MediaReferenceKind::Absolute => Url::parse(form).ok(),
        MediaReferenceKind::SchemeRelative => {
            let base = Url::parse(base_url?.as_ref()).ok()?;
            Url::parse(&format!("{}:{form}", base.scheme())).ok()
        }
    }
}

async fn probe(
    transport: &dyn HeadTransport,
    target: Url,
    instance_id: &InstanceId,
) -> Result<Ownership, HeadTransportError> {
    transport
        .head(&target)
        .await
        .map(|response| Ownership::from_response(&response, instance_id))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ownership {
    Owned,
    Foreign,
    Unknown,
}

impl Ownership {
    fn from_response(response: &HeadResponse, expected: &InstanceId) -> Self {
        match response.instance_headers() {
            [] => Self::Foreign,
            [header] => std::str::from_utf8(header)
                .ok()
                .and_then(|header| InstanceId::from_str(header).ok())
                .map_or(Self::Unknown, |id| {
                    if id == *expected {
                        Self::Owned
                    } else {
                        Self::Foreign
                    }
                }),
            _ => Self::Unknown,
        }
    }
}

fn report_probe_error(error: &HeadTransportError) {
    // Transport failures intentionally fail closed so an unverified row remains live.
    error::report_swallowed(
        host::error::ErrorKind::Internal,
        host::error::ErrorClass::Transient,
        "server.media_ownership.probe",
        host::error::SwallowedSource::Error(error),
    );
}
