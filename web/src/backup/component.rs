use crate::backup::{self, BackUpNow, UpdateSettings};
use crate::error::WebError;
use crate::forms::{self, Field, ValidatedBareInput, ValidatedInput};
use crate::topbar::Topbar;
use crate::warning_revalidation::{BackupWarning, revalidates_warning};
use client::reactive;
use common::MutationOutcome;
use common::backup::{BackupConfig, BackupMode, BackupSchedule, DestinationPath, RetentionCount};
use leptos::prelude::*;
use strum::VariantArray;

#[component]
pub fn BackupSettingsPage() -> impl IntoView {
    let warning = expect_context::<BackupWarning>();
    let update_action = reactive::action_result_if(move || warning.notify(), revalidates_warning);
    // The same typed scope drives the form and its shell warning, so a settled
    // backup-settings mutation re-reads both persisted projections together.
    let settings = reactive::resource(move || warning.track(), backup::get_settings);
    let backup_now = ServerAction::<BackUpNow>::new();

    // The save feedback sits outside both Suspense boundaries so a save error
    // stays visible even when the settings re-read after it fails. The Back Up
    // Now card reads the same persisted settings, because it enables only for a
    // *saved* destination; a failed read is already reported by the form's arm.
    view! {
        <Topbar title="Backup Settings" sub="Operations" />
        <div class="j-scroll">
            <div class="j-settings j-backup-settings">
                <Suspense fallback=|| {
                    view! { <p class="j-loading j-settings-loading">"Loading\u{2026}"</p> }
                }>
                    {move || Suspend::new(async move {
                        match settings.await {
                            Ok(settings) => {
                                backup_settings_form(&settings, update_action).into_any()
                            }
                            Err(error) => {
                                view! { <p class="error j-settings-error">{error.to_string()}</p> }
                                    .into_any()
                            }
                        }
                    })}
                </Suspense>
                {settings_save_feedback(update_action)}
                <Suspense fallback=|| ()>
                    {move || Suspend::new(async move {
                        settings
                            .await
                            .ok()
                            .map(|settings| {
                                backup_now_card(
                                    settings.destination_path.map(|path| path.to_string()),
                                    backup_now,
                                )
                            })
                    })}
                </Suspense>
            </div>
        </div>
    }
}

fn settings_save_feedback(update_action: ServerAction<UpdateSettings>) -> impl IntoView {
    move || {
        update_action
            .value()
            .get()
            .and_then(|result: Result<MutationOutcome<()>, WebError>| {
                match crate::mutation_feedback::classify(
                    result,
                    "Save acknowledgement was lost; reload to verify the settings.",
                ) {
                    crate::mutation_feedback::MutationFeedback::Confirmed(()) => None,
                    crate::mutation_feedback::MutationFeedback::Error(message) => Some(message),
                }
            })
            .map(|error| {
                view! { <p class="error j-settings-error">{error}</p> }
            })
    }
}

/// The destination path is optional and clearable, so it stays direct-bind while sharing
/// the standard ADR-0065 value/error/touch wiring. Extracted so `backup_settings_form`
/// stays within the line budget.
fn backup_destination_field(destination: Field<DestinationPath>) -> impl IntoView {
    view! {
        <label class="j-form-field j-backup-field-wide">
            <span class="j-form-label">"Destination path"</span>
            <ValidatedBareInput<DestinationPath>
                name="destination_path"
                field=destination
                placeholder=Some("/srv/jaunder/backups")
                class=Some("j-form-input")
            />
        </label>
        {forms::validated_error(
            destination.error(),
            Signal::derive(move || destination.is_touched()),
            |msg| view! { <p class="error">{msg}</p> }.into_any(),
        )}
    }
}

fn backup_settings_form(
    settings: &BackupConfig,
    update_action: ServerAction<UpdateSettings>,
) -> impl IntoView {
    // Client-validated fields dispatched directly (no `<ActionForm>`), so the form can carry
    // typed/optional values — the ADR-0065 direct-bind pattern, mirroring
    // `site.rs::site_settings_form`. Destination is optional (empty clears); schedule and
    // retention are required and seeded from the persisted values so an invalid cron or a
    // retention count below 1 disables Save before the request is sent.
    let destination = Field::<DestinationPath>::optional_prefilled(
        settings.destination_path.as_deref().unwrap_or_default(),
    );
    let schedule = Field::<BackupSchedule>::prefilled(&settings.schedule);
    let retention = Field::<RetentionCount>::prefilled(&settings.retention_count.to_string());
    let mode = RwSignal::new(settings.mode);
    let submit = move |_| {
        // The disabled button gates the two required fields valid, so `parsed()` is `Some`.
        if let (Some(schedule), Some(retention_count)) = (schedule.parsed(), retention.parsed()) {
            update_action.dispatch(UpdateSettings {
                // Empty (optional) field → `None`, omitted on the wire → clears the destination;
                // a non-empty value → `Some(DestinationPath)`.
                destination_path: destination.parsed(),
                schedule,
                retention_count,
                mode: mode.get(),
            });
        }
    };
    view! {
        <div class="j-card j-backup-form">
            <div class="j-card-head">
                <div>
                    <h2>"Scheduled Backups"</h2>
                    <div class="j-sub">
                        "Configure where backups are written and how they are retained."
                    </div>
                </div>
            </div>
            <div class="j-form-body j-backup-form-body">
                {backup_destination_field(destination)}
                <p class="j-form-help j-backup-field-wide j-backup-note">
                    "The schedule, and whether scheduled backups run at all, take effect after Jaunder restarts. Destination, retention and mode apply from the next backup."
                </p>
                <ValidatedInput<BackupSchedule>
                    label="Schedule"
                    name="schedule"
                    field=schedule
                    field_class="j-form-field j-backup-field-wide"
                    help="Use a six-field cron expression: second minute hour day-of-month month day-of-week. Example: 0 0 0 * * * runs daily at midnight."
                />
                <ValidatedInput<RetentionCount>
                    label="Retention count"
                    name="retention_count"
                    field=retention
                    input_type="number"
                /> <label class="j-form-field">
                    <span class="j-form-label">"Mode"</span>
                    <select
                        class="j-form-input"
                        name="mode"
                        on:change=move |ev| {
                            mode.set(
                                event_target_value(&ev).parse::<BackupMode>().unwrap_or_default(),
                            );
                        }
                    >
                        {BackupMode::VARIANTS
                            .iter()
                            .copied()
                            .map(|m| {
                                let wire: &'static str = m.into();
                                // `&'static str` (IntoStaticStr) so the option value outlives
                                // the closure — `as_ref()` would borrow the local `m`.
                                view! {
                                    <option value=wire selected=m == settings.mode>
                                        {m.label()}
                                    </option>
                                }
                            })
                            .collect_view()}
                    </select>
                </label>
            </div>
            <div class="j-form-actions">
                <button
                    type="button"
                    class="j-btn is-primary"
                    prop:disabled=move || {
                        !destination.is_valid() || !schedule.is_valid() || !retention.is_valid()
                    }
                    on:click=submit
                >
                    "Save Backup Settings"
                </button>
            </div>
        </div>
    }
}

/// Manual backup: writes one backup to the *saved* destination, so the button
/// stays disabled until a destination has been persisted, not merely typed.
fn backup_now_card(
    destination: Option<String>,
    backup_now: ServerAction<BackUpNow>,
) -> impl IntoView {
    let pending = backup_now.pending();
    let destination_saved = destination.is_some();
    // The body always says where the backup will go (or that nowhere is saved
    // yet), keeping the head/body/actions card shape of the style guide.
    let target = match destination {
        Some(destination) => {
            view! { <p class="j-form-help j-backup-note">"Writes to " <code>{destination}</code></p> }
                .into_any()
        }
        None => {
            view! { <p class="j-form-help j-backup-note">"Save a destination before backing up."</p> }
                .into_any()
        }
    };
    view! {
        <div class="j-card j-backup-now">
            <div class="j-card-head">
                <div>
                    <h2>"Back Up Now"</h2>
                    <div class="j-sub">
                        "Write one backup to the saved destination immediately."
                    </div>
                </div>
            </div>
            <div class="j-form-body">
                {target}
                {move || {
                    backup_now
                        .value()
                        .get()
                        .map(|result: Result<String, WebError>| match result {
                            Ok(path) => {
                                view! {
                                    <p class="success j-backup-note j-backup-written">
                                        "Backup written to " <code>{path}</code>
                                    </p>
                                }
                                    .into_any()
                            }
                            Err(error) => {
                                view! { <p class="error j-backup-note">{error.to_string()}</p> }
                                    .into_any()
                            }
                        })
                }}
            </div>
            <div class="j-form-actions">
                <button
                    type="button"
                    class="j-btn is-primary"
                    prop:disabled=move || !destination_saved || pending.get()
                    on:click=move |_| {
                        backup_now.dispatch(BackUpNow {});
                    }
                >
                    {move || if pending.get() { "Backing Up\u{2026}" } else { "Back Up Now" }}
                </button>
            </div>
        </div>
    }
}

#[component]
pub fn BackupBanner() -> impl IntoView {
    let warning = expect_context::<BackupWarning>();
    let visible = reactive::resource(move || warning.track(), backup::is_warning_visible);
    view! {
        <crate::banner::WarnBanner
            visible=visible
            message="Backups are not configured. Your data is at risk."
            links=vec![("/admin/backups", "Configure Backups"), ("/admin/site", "Site Settings")]
        />
    }
}
