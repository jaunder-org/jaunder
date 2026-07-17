use crate::backup::{get_backup_settings, UpdateBackupSettings};
use crate::error::WebError;
use crate::forms::{Field, ValidatedInput};
use crate::pages::Topbar;
use common::backup::{BackupConfig, BackupMode, BackupSchedule, RetentionCount};
use leptos::prelude::*;
use strum::VariantArray;

#[component]
pub fn BackupSettingsPage() -> impl IntoView {
    let update_action = ServerAction::<UpdateBackupSettings>::new();
    let settings = crate::server_resource(
        move || update_action.version().get(),
        |_| get_backup_settings(),
    );

    view! {
        <Topbar title="Backup Settings" sub="Operations" />
        <div class="j-scroll">
            <div class="j-settings j-backup-settings">
                <Suspense fallback=|| {
                    view! { <p class="j-loading j-settings-loading">"Loading\u{2026}"</p> }
                }>
                    {move || Suspend::new(async move {
                        match settings.await {
                            Ok(settings) => backup_settings_form(settings, update_action).into_any(),
                            Err(error) => {
                                view! { <p class="error j-settings-error">{error.to_string()}</p> }
                                    .into_any()
                            }
                        }
                    })}
                </Suspense>
                {move || {
                    update_action
                        .value()
                        .get()
                        .and_then(|result: Result<(), WebError>| result.err())
                        .map(|error| {
                            view! { <p class="error j-settings-error">{error.to_string()}</p> }
                        })
                }}
            </div>
        </div>
    }
}

// cov:ignore-start
fn backup_settings_form(
    settings: BackupConfig,
    update_action: ServerAction<UpdateBackupSettings>,
) -> impl IntoView {
    // Seed the client-side validated fields from the persisted values (ADR-0065), so an invalid
    // cron or a retention count below 1 disables Save before the request is sent.
    let schedule = Field::<BackupSchedule>::prefilled(&settings.schedule);
    let retention = Field::<RetentionCount>::prefilled(&settings.retention_count.to_string());
    view! {
        <ActionForm action=update_action attr:class="j-card j-backup-form">
            <div class="j-card-head">
                <div>
                    <h2>"Scheduled Backups"</h2>
                    <div class="j-sub">
                        "Configure where backups are written and how they are retained."
                    </div>
                </div>
            </div>
            <div class="j-backup-form-body">
                <label class="j-backup-field j-backup-field-wide">
                    <span class="j-edit-form-label">"Destination Path"</span>
                    <input
                        class="j-backup-input"
                        type="text"
                        name="destination_path"
                        placeholder="/srv/jaunder/backups"
                        prop:value=settings.destination_path.unwrap_or_default()
                    />
                </label>
                <ValidatedInput<
                BackupSchedule,
            >
                    label="Schedule"
                    name="schedule"
                    field=schedule
                    field_class="j-backup-field j-backup-field-wide"
                    class="j-backup-input"
                    help="Use a six-field cron expression: second minute hour day-of-month month day-of-week. Example: 0 0 0 * * * runs daily at midnight."
                />
                <ValidatedInput<
                RetentionCount,
            >
                    label="Retention Count"
                    name="retention_count"
                    field=retention
                    input_type="number"
                    field_class="j-backup-field"
                    class="j-backup-input"
                />
                <label class="j-backup-field">
                    <span class="j-edit-form-label">"Mode"</span>
                    <select class="j-backup-input" name="mode">
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
            <div class="j-backup-form-actions">
                <button
                    type="submit"
                    class="j-btn is-primary"
                    prop:disabled=move || !schedule.is_valid() || !retention.is_valid()
                >
                    "Save Backup Settings"
                </button>
            </div>
        </ActionForm>
    }
}
// cov:ignore-stop
