use crate::backup::{get_backup_settings, BackupSettings, UpdateBackupSettings};
use crate::error::WebError;
use crate::pages::Topbar;
use leptos::prelude::*;

#[component]
pub fn BackupSettingsPage() -> impl IntoView {
    let update_action = ServerAction::<UpdateBackupSettings>::new();
    let settings = Resource::new(
        move || update_action.version().get(),
        |_| get_backup_settings(),
    );

    view! {
        <Topbar title="Backup Settings".to_string() sub="Operations".to_string() />
        <div class="j-scroll">
            <div class="j-settings j-backup-settings">
                <Suspense fallback=|| {
                    view! { <p class="j-settings-loading">"Loading..."</p> }
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

fn backup_settings_form(
    settings: BackupSettings,
    update_action: ServerAction<UpdateBackupSettings>,
) -> impl IntoView {
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
                        prop:value=settings.destination_path
                    />
                </label>
                <label class="j-backup-field j-backup-field-wide">
                    <span class="j-edit-form-label">"Schedule"</span>
                    <input
                        class="j-backup-input"
                        type="text"
                        name="schedule"
                        placeholder="0 0 0 * * *"
                        prop:value=settings.schedule
                        aria-describedby="backup-schedule-help"
                    />
                    <span id="backup-schedule-help" class="j-backup-help">
                        "Use a six-field cron expression: second minute hour day-of-month month day-of-week. Example: 0 0 0 * * * runs daily at midnight."
                    </span>
                </label>
                <label class="j-backup-field">
                    <span class="j-edit-form-label">"Retention Count"</span>
                    <input
                        class="j-backup-input"
                        type="number"
                        min="0"
                        name="retention_count"
                        prop:value=settings.retention_count
                    />
                </label>
                <label class="j-backup-field">
                    <span class="j-edit-form-label">"Mode"</span>
                    <select class="j-backup-input" name="mode" prop:value=settings.mode>
                        <option value="directory">"Directory"</option>
                        <option value="archive">"Archive"</option>
                    </select>
                </label>
            </div>
            <div class="j-backup-form-actions">
                <button type="submit" class="j-btn is-primary">
                    "Save Backup Settings"
                </button>
            </div>
        </ActionForm>
    }
}
