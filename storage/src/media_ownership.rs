use async_trait::async_trait;
use common::tagged_url::BaseUrl;

use crate::InstanceId;
use crate::posts::store::{
    MediaReferenceEvidence, PersistedMediaReference, ProvenForeignReference,
};

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
