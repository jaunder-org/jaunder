use crate::backup::{get_backup_settings, UpdateBackupSettings};
use crate::error::WebError;
use crate::forms::{Field, ValidatedInput};
use crate::pages::Topbar;
use common::backup::{BackupConfig, BackupMode, BackupSchedule};
use leptos::prelude::*;

#[component]
pub fn BackupSettingsPage() -> impl IntoView {
    let update_action = ServerAction::<UpdateBackupSettings>::new();
    let settings = crate::server_resource(
        move || update_action.version().get(),
        |_| get_backup_settings(),
    );

    view! {
        <Topbar title="Backup Settings".to_string() sub="Operations".to_string() />
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
    let mode_str = match settings.mode {
        BackupMode::Directory => "directory",
        BackupMode::Archive => "archive",
        // cov:ignore-stop
    };
    // cov:ignore-start
    // Seed the client-side validated schedule field from the persisted value (ADR-0065), so an
    // invalid cron edit disables Save before the request is sent (Deref: &BackupSchedule → &str).
    let schedule = Field::<BackupSchedule>::prefilled(&settings.schedule);
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
                <label class="j-backup-field">
                    <span class="j-edit-form-label">"Retention Count"</span>
                    <input
                        class="j-backup-input"
                        type="number"
                        min="0"
                        name="retention_count"
                        prop:value=settings.retention_count.to_string()
                    />
                </label>
                <label class="j-backup-field">
                    <span class="j-edit-form-label">"Mode"</span>
                    <select class="j-backup-input" name="mode" prop:value=mode_str>
                        <option value="directory">"Directory"</option>
                        <option value="archive">"Archive"</option>
                    </select>
                </label>
            </div>
            <div class="j-backup-form-actions">
                <button
                    type="submit"
                    class="j-btn is-primary"
                    prop:disabled=move || !schedule.is_valid()
                >
                    "Save Backup Settings"
                </button>
            </div>
        </ActionForm>
    }
}
// cov:ignore-stop
