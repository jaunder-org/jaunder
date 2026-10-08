use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::production_baseline::{PackageIdentity, ResolvedRevision, StorageBackend};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SourcePin {
    commit: String,
    locked_ref: String,
}
impl SourcePin {
    pub(crate) fn admit_current(root: &Path) -> Result<Self> {
        let root = fs::canonicalize(root).context("canonicalizing qualification source root")?;
        let top = git_output(&root, &["rev-parse", "--show-toplevel"])?;
        if fs::canonicalize(top)? != root {
            bail!("qualification source must be the current repository root");
        }
        if !git_output(&root, &["status", "--porcelain"])?.is_empty() {
            bail!("qualification source requires a clean committed checkout");
        }
        let commit = git_output(&root, &["rev-parse", "HEAD"])?;
        valid_hex(&commit, 40, "qualification source HEAD")?;
        if git_output(&root, &["cat-file", "-t", &commit])? != "commit" {
            bail!("qualification source HEAD is not a commit");
        }
        let file_url = Url::from_file_path(&root)
            .map_err(|_| anyhow::anyhow!("qualification source path cannot form a file URL"))?;
        let file_url = file_url
            .as_str()
            .strip_prefix("file:")
            .context("qualification source file URL is malformed")?;
        Ok(Self {
            commit: commit.clone(),
            locked_ref: format!("git+file:{file_url}?rev={commit}"),
        })
    }
    /// The browser executes tracked TypeScript from the live checkout. Re-admit
    /// the complete tree around execution and publication, not just its entry
    /// point: changed imports, configuration, HEAD, or untracked inputs must all
    /// refuse evidence attributed to the original immutable source.
    pub(crate) fn verify_current(&self, root: &Path) -> Result<()> {
        if Self::admit_current(root)? != *self {
            bail!("qualification checkout drifted from its admitted source");
        }
        Ok(())
    }
    pub(crate) fn commit(&self) -> &str {
        &self.commit
    }
    pub(crate) fn locked_ref(&self) -> &str {
        &self.locked_ref
    }
}
fn git_output(root: &Path, args: &[&str]) -> Result<String> {
    let output = crate::git::at(root)
        .args(args)
        .output()
        .context("reading qualification source Git state")?;
    if !output.status.success() {
        bail!("qualification source git {args:?} failed")
    };
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}
fn valid_hex(value: &str, length: usize, label: &str) -> Result<()> {
    if value.len() != length || !value.bytes().all(|v| v.is_ascii_hexdigit()) {
        bail!("{label} must be {length} hexadecimal characters")
    };
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum QualificationFixture {
    A,
    #[serde(rename = "b-app")]
    BApplication,
    BTheme,
}
impl QualificationFixture {
    pub(crate) const fn token(self) -> &'static str {
        match self {
            Self::A => "a",
            Self::BApplication => "b-app",
            Self::BTheme => "b-theme",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum QualificationTransition {
    Application,
    Studio,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum QualificationSurface {
    Local,
    AuthorPermalink,
    Home,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum QualificationPhase {
    Preparation,
    Create,
    AWarm,
    AppB,
    RollbackA,
    ThemeB,
    Restored,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum CacheEvidence {
    RequestServedFromCache,
    RequestServedFromDiskCache,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AssetExpectation {
    pub role: String,
    pub url: String,
    pub digest: String,
    pub revision: Option<String>,
    pub sha256: String,
    pub mime: String,
    pub bytes: u64,
    pub etag: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PublicPresentationExpectation {
    pub stylesheet: AssetExpectation,
    pub revision: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SurfaceExpectation {
    pub surface: QualificationSurface,
    pub application: AssetExpectation,
    pub public_presentation: Option<PublicPresentationExpectation>,
    pub topbar_border_color: Option<String>,
    pub studio_accent: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct QualificationRequest {
    pub sequence: u32,
    pub phase: QualificationPhase,
    pub fixture: QualificationFixture,
    pub transition: QualificationTransition,
    pub backend: StorageBackend,
    pub source: ResolvedRevision,
    pub package: PackageIdentity,
    pub expected_surfaces: Vec<SurfaceExpectation>,
    pub requested_cache_urls: Vec<String>,
    pub requested_http_assets: Vec<AssetExpectation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed_process: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SurfaceObservation {
    pub surface: QualificationSurface,
    pub application_url: String,
    pub application_digest: String,
    pub public_presentation: Option<PublicPresentationObservation>,
    pub topbar_border_color: Option<String>,
    pub studio_accent: Option<String>,
    pub public_links: u32,
    pub staged_package_links: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PublicPresentationObservation {
    pub stylesheet_url: String,
    pub revision: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CachedAssetObservation {
    pub url: String,
    pub evidence: CacheEvidence,
    pub document_path: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OldAssetHttpObservation {
    pub role: String,
    pub url: String,
    pub digest: String,
    pub sha256: String,
    pub mime: String,
    pub bytes: u64,
    pub etag: String,
    pub cache_control: String,
    pub status_200: u16,
    pub status_304: u16,
    pub body_bytes_304: u64,
    pub readable_after_restore: Option<bool>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct QualificationResult {
    pub sequence: u32,
    pub phase: QualificationPhase,
    pub fixture: QualificationFixture,
    pub transition: QualificationTransition,
    pub backend: StorageBackend,
    pub anonymous_context_id: String,
    pub authenticated_context_id: String,
    pub surfaces: Vec<SurfaceObservation>,
    pub cached_assets: Vec<CachedAssetObservation>,
    pub old_assets: Vec<OldAssetHttpObservation>,
}

impl QualificationRequest {
    pub(crate) fn validate(&self) -> Result<()> {
        valid_hex(&self.source.commit, 40, "source commit")?;
        if self.source.flake_ref
            != format!(
                "git+file:{}?rev={}",
                self.source
                    .flake_ref
                    .strip_prefix("git+file:")
                    .context("source flake must be git+file")?
                    .split("?rev=")
                    .next()
                    .unwrap_or_default(),
                self.source.commit
            )
        {
            bail!("source flake reference is not locked to source commit");
        }
        if self.package.installable.is_empty()
            || self.package.derivation.is_empty()
            || self.package.output_path.is_empty()
            || self.package.nar_hash.is_empty()
        {
            bail!("package identity is incomplete")
        };
        valid_hex(
            &self.package.executable_sha256,
            64,
            "package executable SHA-256",
        )?;
        if !valid_combo(self.phase, self.fixture, self.transition) {
            bail!("qualification phase, fixture, and transition are inconsistent")
        }
        let required = required_surfaces(self.phase, self.transition);
        if self
            .expected_surfaces
            .iter()
            .map(|v| v.surface)
            .collect::<BTreeSet<_>>()
            != required
            || self.expected_surfaces.len() != required.len()
        {
            bail!("request has an invalid expected surface set")
        }
        for surface in &self.expected_surfaces {
            validate_asset(&surface.application, "application", false)?;
            if surface.surface == QualificationSurface::Home {
                if surface.public_presentation.is_some() || surface.studio_accent.is_some() {
                    bail!("Home expectation cannot contain public presentation")
                }
            } else {
                let public = surface
                    .public_presentation
                    .as_ref()
                    .context("public surface requires selected Studio presentation")?;
                validate_asset(&public.stylesheet, "studio", true)?;
                if public.revision != public.stylesheet.revision.clone().unwrap_or_default()
                    || !valid_computed_expectation(&surface.studio_accent, self.phase)
                {
                    bail!("public Studio expectation is incomplete")
                }
            }
            if !valid_computed_expectation(&surface.topbar_border_color, self.phase) {
                bail!("surface requires exact application computed color")
            }
        }
        exact_assets(&self.requested_http_assets)?;
        if self
            .requested_cache_urls
            .iter()
            .collect::<BTreeSet<_>>()
            .len()
            != self.requested_cache_urls.len()
            || self.requested_cache_urls.iter().any(|v| v.is_empty())
        {
            bail!("requested cache URL set is invalid")
        };
        Ok(())
    }
}
// Create captures the ordinary release's actual computed values. Every later
// phase must compare against that frozen baseline or the bounded B declaration.
fn valid_computed_expectation(value: &Option<String>, phase: QualificationPhase) -> bool {
    match value {
        Some(value) => !value.is_empty(),
        None => phase == QualificationPhase::Create,
    }
}

fn matches_computed(observed: &Option<String>, expected: &Option<String>, capture: bool) -> bool {
    match expected {
        Some(expected) => observed.as_ref() == Some(expected),
        None if capture => observed.as_ref().is_some_and(|value| !value.is_empty()),
        None => observed.is_none(),
    }
}
fn valid_combo(p: QualificationPhase, f: QualificationFixture, t: QualificationTransition) -> bool {
    match p {
        QualificationPhase::Preparation
        | QualificationPhase::Create
        | QualificationPhase::AWarm => f == QualificationFixture::A,
        QualificationPhase::AppB => {
            f == QualificationFixture::BApplication && t == QualificationTransition::Application
        }
        QualificationPhase::ThemeB => {
            f == QualificationFixture::BTheme && t == QualificationTransition::Studio
        }
        QualificationPhase::RollbackA => f == QualificationFixture::A,
        QualificationPhase::Restored => f == QualificationFixture::A,
    }
}
fn required_surfaces(
    p: QualificationPhase,
    t: QualificationTransition,
) -> BTreeSet<QualificationSurface> {
    match p {
        QualificationPhase::Preparation
        | QualificationPhase::Create
        | QualificationPhase::AWarm
        | QualificationPhase::Restored => BTreeSet::from([
            QualificationSurface::Local,
            QualificationSurface::AuthorPermalink,
            QualificationSurface::Home,
        ]),
        _ if t == QualificationTransition::Application => {
            BTreeSet::from([QualificationSurface::Local, QualificationSurface::Home])
        }
        _ => BTreeSet::from([
            QualificationSurface::Local,
            QualificationSurface::AuthorPermalink,
        ]),
    }
}
fn validate_asset(a: &AssetExpectation, role: &str, revision: bool) -> Result<()> {
    if a.role != role
        || a.bytes == 0
        || a.mime != "text/css; charset=utf-8"
        || a.url != format!("/theme/{}", a.digest)
        || a.etag.is_empty()
    {
        bail!("invalid {role} asset expectation")
    };
    valid_hex(&a.digest, 64, "asset digest")?;
    valid_hex(&a.sha256, 64, "asset SHA-256")?;
    if revision && a.revision.as_deref().unwrap_or("").is_empty() {
        bail!("Studio asset requires revision")
    };
    Ok(())
}
fn exact_assets(assets: &[AssetExpectation]) -> Result<()> {
    let mut urls = BTreeSet::new();
    for asset in assets {
        validate_asset(asset, &asset.role, asset.revision.is_some())?;
        if !urls.insert(asset.url.as_str()) {
            bail!("HTTP asset expectations must have unique URLs")
        }
    }
    Ok(())
}
impl QualificationResult {
    pub(crate) fn validate_for(&self, request: &QualificationRequest) -> Result<()> {
        request.validate()?;
        if self.sequence != request.sequence
            || self.phase != request.phase
            || self.fixture != request.fixture
            || self.transition != request.transition
            || self.backend != request.backend
        {
            bail!("qualification result identity does not match request")
        };
        if self.anonymous_context_id.is_empty() || self.authenticated_context_id.is_empty() {
            bail!("result omits retained browser context identity")
        };
        if self
            .surfaces
            .iter()
            .map(|v| v.surface)
            .collect::<BTreeSet<_>>()
            != request
                .expected_surfaces
                .iter()
                .map(|v| v.surface)
                .collect()
            || self.surfaces.len() != request.expected_surfaces.len()
        {
            bail!("result surface set differs from request")
        };
        for observed in &self.surfaces {
            let expected = request
                .expected_surfaces
                .iter()
                .find(|v| v.surface == observed.surface)
                .unwrap();
            if observed.application_url != expected.application.url
                || observed.application_digest != expected.application.digest
                || !matches_computed(
                    &observed.topbar_border_color,
                    &expected.topbar_border_color,
                    self.phase == QualificationPhase::Create,
                )
                || !matches_computed(
                    &observed.studio_accent,
                    &expected.studio_accent,
                    self.phase == QualificationPhase::Create
                        && expected.public_presentation.is_some(),
                )
            {
                bail!("surface observation disagrees with request")
            };
            match (&expected.public_presentation, &observed.public_presentation) {
                (None, None) => {
                    if observed.public_links != 0 || observed.staged_package_links != 0 {
                        bail!("Home has public package links")
                    }
                }
                (Some(e), Some(o))
                    if o.stylesheet_url == e.stylesheet.url
                        && o.revision == e.revision
                        && observed.public_links == 1
                        && observed.staged_package_links == 0 => {}
                _ => bail!("public presentation disagrees with request"),
            }
        }
        let cache = self
            .cached_assets
            .iter()
            .map(|v| v.url.as_str())
            .collect::<BTreeSet<_>>();
        let expected = request
            .requested_cache_urls
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        if cache != expected
            || cache.len() != self.cached_assets.len()
            || self
                .cached_assets
                .iter()
                .any(|v| v.document_path.is_empty())
        {
            bail!("cache observations differ from requested URLs")
        };
        let http = self
            .old_assets
            .iter()
            .map(|v| v.url.as_str())
            .collect::<BTreeSet<_>>();
        let wanted = request
            .requested_http_assets
            .iter()
            .map(|v| v.url.as_str())
            .collect::<BTreeSet<_>>();
        if http != wanted || http.len() != self.old_assets.len() {
            bail!("HTTP observations differ from requested assets")
        };
        for old in &self.old_assets {
            let e = request
                .requested_http_assets
                .iter()
                .find(|v| v.url == old.url)
                .unwrap();
            if old.role != e.role
                || old.digest != e.digest
                || old.sha256 != e.sha256
                || old.mime != e.mime
                || old.bytes != e.bytes
                || old.etag != e.etag
                || old.cache_control != "public, max-age=31536000, immutable"
                || old.status_200 != 200
                || old.status_304 != 304
                || old.body_bytes_304 != 0
            {
                bail!("HTTP observation disagrees with requested asset")
            };
            if self.phase == QualificationPhase::Restored {
                if old.readable_after_restore != Some(true) {
                    bail!("restored phase requires retained readability")
                }
            } else if old.readable_after_restore.is_some() {
                bail!("pre-restore phase must not claim restored readability")
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    fn git(root: &Path, args: &[&str]) {
        assert!(crate::git::at(root).args(args).status().unwrap().success())
    }
    fn repo() -> tempfile::TempDir {
        let t = tempdir().unwrap();
        let r = t.path();
        git(r, &["init"]);
        git(r, &["config", "user.email", "a@b.c"]);
        git(r, &["config", "user.name", "a"]);
        fs::write(r.join("x"), "x").unwrap();
        git(r, &["add", "x"]);
        git(r, &["commit", "-m", "x"]);
        t
    }
    #[test]
    fn pin_requires_clean_committed_source() {
        let r = repo();
        let pin = SourcePin::admit_current(r.path()).unwrap();
        assert!(pin.locked_ref().ends_with(pin.commit()));
        assert!(pin.verify_current(r.path()).is_ok());
        fs::write(r.path().join("dirty"), "x").unwrap();
        assert!(SourcePin::admit_current(r.path()).is_err());
        assert!(pin.verify_current(r.path()).is_err());
    }
    #[test]
    fn harness_source_drift_refuses_the_admitted_pin() {
        let r = repo();
        let pin = SourcePin::admit_current(r.path()).unwrap();
        fs::write(r.path().join("x"), "changed harness import").unwrap();
        assert!(pin.verify_current(r.path()).is_err());
    }
    #[test]
    fn clean_new_head_still_refuses_the_original_pin() {
        let r = repo();
        let pin = SourcePin::admit_current(r.path()).unwrap();
        fs::write(r.path().join("x"), "changed harness import").unwrap();
        git(r.path(), &["add", "x"]);
        git(r.path(), &["commit", "-m", "changed harness"]);
        assert!(SourcePin::admit_current(r.path()).is_ok());
        assert!(pin.verify_current(r.path()).is_err());
    }
    fn hex(c: char) -> String {
        c.to_string().repeat(64)
    }
    fn asset(role: &str, c: char, revision: bool) -> AssetExpectation {
        let d = hex(c);
        AssetExpectation {
            role: role.into(),
            url: format!("/theme/{d}"),
            digest: d.clone(),
            revision: revision.then(|| hex('e')),
            sha256: hex('f'),
            mime: "text/css; charset=utf-8".into(),
            bytes: 1,
            etag: "\"tag\"".into(),
        }
    }
    fn request(
        phase: QualificationPhase,
        fixture: QualificationFixture,
        transition: QualificationTransition,
    ) -> QualificationRequest {
        let app = asset("application", 'a', false);
        let studio = asset("studio", 'b', true);
        let set = required_surfaces(phase, transition)
            .into_iter()
            .map(|surface| SurfaceExpectation {
                surface,
                application: app.clone(),
                public_presentation: (surface != QualificationSurface::Home).then(|| {
                    PublicPresentationExpectation {
                        stylesheet: studio.clone(),
                        revision: studio.revision.clone().unwrap(),
                    }
                }),
                topbar_border_color: Some("rgb(79, 70, 229)".into()),
                studio_accent: (surface != QualificationSurface::Home).then(|| "#0f766e".into()),
            })
            .collect();
        QualificationRequest {
            sequence: 1,
            phase,
            fixture,
            transition,
            backend: StorageBackend::Sqlite,
            source: ResolvedRevision {
                commit: "c".repeat(40),
                flake_ref: format!("git+file:///x?rev={}", "c".repeat(40)),
            },
            package: PackageIdentity {
                installable: "i".into(),
                derivation: "d".into(),
                output_path: "o".into(),
                nar_hash: "n".into(),
                executable_sha256: hex('d'),
            },
            expected_surfaces: set,
            requested_cache_urls: if phase == QualificationPhase::AWarm {
                vec![app.url.clone(), studio.url.clone()]
            } else {
                vec![]
            },
            requested_http_assets: vec![app, studio],
            seed_process: None,
        }
    }
    fn result(r: &QualificationRequest) -> QualificationResult {
        QualificationResult {
            sequence: 1,
            phase: r.phase,
            fixture: r.fixture,
            transition: r.transition,
            backend: r.backend,
            anonymous_context_id: "anon".into(),
            authenticated_context_id: "auth".into(),
            surfaces: r
                .expected_surfaces
                .iter()
                .map(|e| SurfaceObservation {
                    surface: e.surface,
                    application_url: e.application.url.clone(),
                    application_digest: e.application.digest.clone(),
                    public_presentation: e.public_presentation.as_ref().map(|p| {
                        PublicPresentationObservation {
                            stylesheet_url: p.stylesheet.url.clone(),
                            revision: p.revision.clone(),
                        }
                    }),
                    topbar_border_color: e.topbar_border_color.clone(),
                    studio_accent: e.studio_accent.clone(),
                    public_links: u32::from(e.public_presentation.is_some()),
                    staged_package_links: 0,
                })
                .collect(),
            cached_assets: r
                .requested_cache_urls
                .iter()
                .map(|url| CachedAssetObservation {
                    url: url.clone(),
                    evidence: CacheEvidence::RequestServedFromCache,
                    document_path: "/".into(),
                })
                .collect(),
            old_assets: r
                .requested_http_assets
                .iter()
                .map(|e| OldAssetHttpObservation {
                    role: e.role.clone(),
                    url: e.url.clone(),
                    digest: e.digest.clone(),
                    sha256: e.sha256.clone(),
                    mime: e.mime.clone(),
                    bytes: e.bytes,
                    etag: e.etag.clone(),
                    cache_control: "public, max-age=31536000, immutable".into(),
                    status_200: 200,
                    status_304: 304,
                    body_bytes_304: 0,
                    readable_after_restore: (r.phase == QualificationPhase::Restored)
                        .then_some(true),
                })
                .collect(),
        }
    }
    #[test]
    fn protocol_accepts_application_studio_and_cold_controls() {
        for (p, f, t) in [
            (
                QualificationPhase::Create,
                QualificationFixture::A,
                QualificationTransition::Application,
            ),
            (
                QualificationPhase::AppB,
                QualificationFixture::BApplication,
                QualificationTransition::Application,
            ),
            (
                QualificationPhase::ThemeB,
                QualificationFixture::BTheme,
                QualificationTransition::Studio,
            ),
            (
                QualificationPhase::Restored,
                QualificationFixture::A,
                QualificationTransition::Studio,
            ),
        ] {
            let r = request(p, f, t);
            assert!(result(&r).validate_for(&r).is_ok())
        }
    }
    #[test]
    fn protocol_rejects_false_positive_observations() {
        let r = request(
            QualificationPhase::AppB,
            QualificationFixture::BApplication,
            QualificationTransition::Application,
        );
        let mut x = result(&r);
        x.surfaces[0].application_url = "/theme/".to_owned();
        assert!(x.validate_for(&r).is_err());
        let mut x = result(&r);
        x.surfaces[0].studio_accent = Some("#000000".into());
        assert!(x.validate_for(&r).is_err());
        let mut x = result(&r);
        x.cached_assets.push(CachedAssetObservation {
            url: "/x".into(),
            evidence: CacheEvidence::RequestServedFromCache,
            document_path: "/".into(),
        });
        assert!(x.validate_for(&r).is_err());
        let mut x = result(&r);
        x.old_assets[0].etag = "bad".into();
        assert!(x.validate_for(&r).is_err());
        let mut x = result(&r);
        x.old_assets[0].readable_after_restore = Some(true);
        assert!(x.validate_for(&r).is_err())
    }
    #[test]
    fn retained_http_versions_share_roles_but_not_urls() {
        let mut request = request(
            QualificationPhase::AppB,
            QualificationFixture::BApplication,
            QualificationTransition::Application,
        );
        request
            .requested_http_assets
            .push(asset("application", 'c', false));
        assert!(result(&request).validate_for(&request).is_ok());
        request
            .requested_http_assets
            .push(request.requested_http_assets[0].clone());
        assert!(request.validate().is_err());
    }

    #[test]
    fn create_captures_actual_colors_before_frozen_comparisons() {
        let mut request = request(
            QualificationPhase::Create,
            QualificationFixture::A,
            QualificationTransition::Application,
        );
        let mut observed = result(&request);
        for surface in &mut request.expected_surfaces {
            surface.topbar_border_color = None;
            surface.studio_accent = None;
        }
        assert!(observed.validate_for(&request).is_ok());
        observed.surfaces[0].topbar_border_color = None;
        assert!(observed.validate_for(&request).is_err());
        request.phase = QualificationPhase::AWarm;
        assert!(request.validate().is_err());
    }

    #[test]
    fn request_is_closed_and_rejects_invalid_identity() {
        let mut r = request(
            QualificationPhase::AppB,
            QualificationFixture::BApplication,
            QualificationTransition::Application,
        );
        r.source.flake_ref = "path:/x".into();
        assert!(r.validate().is_err());
        let r = request(
            QualificationPhase::ThemeB,
            QualificationFixture::BApplication,
            QualificationTransition::Studio,
        );
        assert!(r.validate().is_err());
    }
}
