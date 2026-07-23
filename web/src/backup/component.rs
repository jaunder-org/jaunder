use crate::backup::{backup_warning_visible, get_backup_settings, UpdateBackupSettings};
use crate::error::WebError;
use crate::forms::{Field, ValidatedInput};
use crate::topbar::Topbar;
use common::backup::{BackupConfig, BackupMode, BackupSchedule, DestinationPath, RetentionCount};
use leptos::prelude::*;
use strum::VariantArray;

#[component]
pub fn BackupSettingsPage() -> impl IntoView {
    let update_action = ServerAction::<UpdateBackupSettings>::new();
    let settings = Resource::new(
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
/// The destination path is optional and clearable, so it is a direct-bind `Field` (not a
/// `ValidatedInput`) — that keeps the placeholder and bespoke classes the shared component
/// can't yet express (#450). Extracted so `backup_settings_form` stays within the line budget.
fn backup_destination_field(destination: Field<DestinationPath>) -> impl IntoView {
    view! {
        <label class="j-backup-field j-backup-field-wide">
            <span class="j-edit-form-label">"Destination Path"</span>
            <input
                class="j-backup-input"
                type="text"
                name="destination_path"
                placeholder="/srv/jaunder/backups"
                prop:value=destination.value
                on:input=move |ev| {
                    let v = event_target_value(&ev);
                    destination.value.set(v.clone());
                    destination.error.set(destination.error_for(&v));
                }
                on:blur=move |_| destination.touch()
            />
        </label>
        {move || {
            destination
                .is_touched()
                .then(|| destination.error.get())
                .flatten()
                .map(|msg| view! { <p class="error">{msg}</p> })
        }}
    }
}

fn backup_settings_form(
    settings: &BackupConfig,
    update_action: ServerAction<UpdateBackupSettings>,
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
            update_action.dispatch(UpdateBackupSettings {
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
            <div class="j-backup-form-body">
                {backup_destination_field(destination)}
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
                /> <label class="j-backup-field">
                    <span class="j-edit-form-label">"Mode"</span>
                    <select
                        class="j-backup-input"
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
            <div class="j-backup-form-actions">
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
// cov:ignore-stop

#[component]
pub fn BackupBanner() -> impl IntoView {
    let visible = Resource::new(|| (), |()| backup_warning_visible());

    view! {
        <Suspense fallback=|| ()>
            {move || Suspend::new(async move {
                match visible.await {
                    Ok(true) => {
                        view! {
                            <div class="j-backup-banner" role="alert">
                                <span>"Backups are not configured. Your data is at risk."</span>
                                <div>
                                    <a href="/admin/backups">"Configure Backups"</a>
                                    <a href="/admin/site">"Site Settings"</a>
                                </div>
                            </div>
                        }
                            .into_any()
                    }
                    _ => ().into_any(),
                }
            })}
        </Suspense>
    }
}
