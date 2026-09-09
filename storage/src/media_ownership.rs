use std::collections::BTreeSet;
use std::sync::Arc;

use async_trait::async_trait;
use common::tagged_url::BaseUrl;

use crate::posts::media::{
    MediaReferenceEvidence, PersistedMediaReference, ProvenForeignReference,
};
use crate::{InstanceId, SiteConfigStorage};
use common::media::{MediaRef, MediaReference};
#[cfg(any(test, feature = "test-utils"))]
use tokio::sync::Notify;

/// Resolver-only capability for recording an exact foreign result.
///
/// Storage creates this capability at the resolver invocation boundary. Consumers
/// can observe evidence but cannot manufacture a capability or foreign proof.
///
/// ```
/// # use storage::{ForeignEvidenceSink, ProvenForeignReference};
/// let _sink_type = std::any::type_name::<ForeignEvidenceSink>();
/// let _proof_type = std::any::type_name::<ProvenForeignReference>();
/// ```
///
/// ```compile_fail
/// # use storage::{ForeignEvidenceSink, ProvenForeignReference};
/// let _ = ForeignEvidenceSink::new;
/// let _ = ProvenForeignReference::new;
/// ```
///
/// Both constructors are intentionally unavailable to arbitrary callers; only
/// [`resolve_media_reference_ownership`] can mint the sink passed to a resolver.
pub struct ForeignEvidenceSink {
    evidence: MediaReferenceEvidence,
}

impl ForeignEvidenceSink {
    fn new(instance_id: InstanceId) -> Self {
        Self {
            evidence: MediaReferenceEvidence::new(instance_id),
        }
    }

    /// Records one foreign result for the resolver's identity snapshot.
    pub fn prove_foreign(&mut self, reference: PersistedMediaReference) {
        let proof =
            ProvenForeignReference::new(reference, self.evidence.expected_instance_id().clone());
        self.evidence.insert(proof);
    }

    #[must_use]
    pub fn finish(self) -> MediaReferenceEvidence {
        self.evidence
    }
}

/// Resolver-only capability for materializing exact locally served media.
///
/// The contained identities are deliberately private: only a resolver invoked by
/// [`resolve_local_media_references`] can mint this proof from rendered output.
#[derive(Default)]
pub struct ProvenLocalMediaRefs {
    media: BTreeSet<MediaRef>,
}

impl ProvenLocalMediaRefs {
    fn new(media: BTreeSet<MediaRef>) -> Self {
        Self { media }
    }

    pub(crate) fn media(&self) -> &BTreeSet<MediaRef> {
        &self.media
    }
}

/// Resolver-only sink for exact local identities.
pub struct LocalMediaSink {
    media: BTreeSet<MediaRef>,
}

impl LocalMediaSink {
    fn new() -> Self {
        Self {
            media: BTreeSet::new(),
        }
    }

    /// Adds one exact identity proved local by the resolver.
    pub fn prove_local(&mut self, media: MediaRef) {
        self.media.insert(media);
    }

    #[must_use]
    pub fn finish(self) -> ProvenLocalMediaRefs {
        ProvenLocalMediaRefs::new(self.media)
    }
}

/// Per-post-service coordination point reached after a writer acquires its media
/// content lock and before it begins its storage transaction.
#[cfg(any(test, feature = "test-utils"))]
pub struct PostWriteGate {
    arrived: Notify,
    resume: Notify,
}

#[cfg(any(test, feature = "test-utils"))]
impl PostWriteGate {
    #[must_use]
    pub fn new() -> Self {
        Self {
            arrived: Notify::new(),
            resume: Notify::new(),
        }
    }

    pub async fn wait_until_writer_holds_lock(&self) {
        self.arrived.notified().await;
    }

    pub fn release(&self) {
        self.resume.notify_one();
    }

    pub(crate) async fn pause(&self) {
        self.arrived.notify_one();
        self.resume.notified().await;
    }
}

#[cfg(any(test, feature = "test-utils"))]
// cov:ignore-start — test-only synchronization gate Default implementation has no production caller
impl Default for PostWriteGate {
    fn default() -> Self {
        Self::new()
    }
}
// cov:ignore-stop

/// Composition-root dependency for post content materialization.
#[derive(Clone)]
pub struct PostMediaOwnership {
    resolver: Arc<dyn MediaReferenceOwnershipResolver>,
    instance_id: InstanceId,
    site_config: Arc<dyn SiteConfigStorage>,
    #[cfg(any(test, feature = "test-utils"))]
    post_write_gate: Option<Arc<PostWriteGate>>,
}

impl PostMediaOwnership {
    #[must_use]
    pub fn new(
        resolver: Arc<dyn MediaReferenceOwnershipResolver>,
        instance_id: InstanceId,
        site_config: Arc<dyn SiteConfigStorage>,
    ) -> Self {
        Self {
            resolver,
            instance_id,
            site_config,
            #[cfg(any(test, feature = "test-utils"))]
            post_write_gate: None,
        }
    }

    /// Installs a per-service test gate after the writer holds its media lock.
    #[cfg(any(test, feature = "test-utils"))]
    #[must_use]
    pub fn with_post_write_gate_for_test(mut self, gate: Arc<PostWriteGate>) -> Self {
        self.post_write_gate = Some(gate);
        self
    }

    #[cfg(any(test, feature = "test-utils"))]
    pub(crate) async fn pause_after_media_lock_for_test(&self) {
        if let Some(gate) = &self.post_write_gate {
            gate.pause().await;
        }
    }

    /// Resolves rendered references into an opaque local-media capability.
    ///
    /// # Errors
    ///
    /// Returns a storage error when the canonical site identity cannot be
    /// loaded.
    pub async fn resolve(
        &self,
        references: &[MediaReference],
    ) -> Result<ProvenLocalMediaRefs, sqlx::Error> {
        let identity = self.site_config.get_identity().await?;
        Ok(resolve_local_media_references(
            self.resolver.as_ref(),
            references,
            &self.instance_id,
            identity.base_url.as_ref(),
        )
        .await)
    }
}

/// Resolves rendered references at the pre-write boundary.
pub async fn resolve_local_media_references(
    resolver: &dyn MediaReferenceOwnershipResolver,
    references: &[MediaReference],
    instance_id: &InstanceId,
    base_url: Option<&BaseUrl>,
) -> ProvenLocalMediaRefs {
    resolver
        .resolve_local(references, instance_id, base_url, LocalMediaSink::new())
        .await
}

/// Resolves live foreign ownership evidence for the complete global set of
/// persisted media-reference rows.
///
/// Implementations must fail closed: rows with unavailable or ambiguous
/// ownership results are omitted from the evidence, so storage keeps them as
/// references. The abstraction belongs here because the web and `AtomPub`
/// surfaces consume it alongside the persisted row and evidence types, while
/// the live network implementation remains server-owned.
#[async_trait]
pub trait MediaReferenceOwnershipResolver: Send + Sync {
    /// Resolves foreign-reference evidence under one instance-identity and site
    /// identity snapshot using storage's unforgeable proof capability.
    async fn resolve(
        &self,
        references: &[PersistedMediaReference],
        instance_id: &InstanceId,
        base_url: Option<&BaseUrl>,
        foreign: ForeignEvidenceSink,
    ) -> MediaReferenceEvidence;

    /// Resolves rendered references into unforgeable local-ownership proof.
    async fn resolve_local(
        &self,
        references: &[MediaReference],
        instance_id: &InstanceId,
        base_url: Option<&BaseUrl>,
        local: LocalMediaSink,
    ) -> ProvenLocalMediaRefs;
}

/// Invokes a resolver with the only capability that can create foreign evidence.
pub async fn resolve_media_reference_ownership(
    resolver: &dyn MediaReferenceOwnershipResolver,
    references: &[PersistedMediaReference],
    instance_id: &InstanceId,
    base_url: Option<&BaseUrl>,
) -> MediaReferenceEvidence {
    resolver
        .resolve(
            references,
            instance_id,
            base_url,
            ForeignEvidenceSink::new(instance_id.clone()),
        )
        .await
}
