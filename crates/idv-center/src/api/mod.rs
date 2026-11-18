mod admins;
mod agent;
mod agent_commands;
mod agent_packages;
mod assist;
pub(crate) mod audit;
mod auth;
mod campuses;
mod clients;
mod command_presets;
mod dashboard;
mod delta_images;
mod edge_agent;
mod edge_bindings;
mod edges;
mod images;
mod labs;
mod modes;
pub mod ops;
mod pe_sessions;
mod power;
mod remote;
mod software;
pub mod tasks;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::{delete, get, post, put};

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/auth/login", post(auth::login))
        .route("/api/v1/auth/me", get(auth::me).put(auth::update_me))
        .route("/api/v1/dashboard", get(dashboard::stats))
        .route(
            "/api/v1/campuses",
            get(campuses::list).post(campuses::create),
        )
        .route(
            "/api/v1/campuses/{id}",
            get(campuses::get)
                .put(campuses::update)
                .delete(campuses::delete),
        )
        .route("/api/v1/labs", get(labs::list).post(labs::create))
        .route(
            "/api/v1/labs/{id}",
            get(labs::get).put(labs::update).delete(labs::delete),
        )
        .route("/api/v1/clients", get(clients::list).post(clients::create))
        .route("/api/v1/clients/bulk", post(clients::bulk_create))
        .route("/api/v1/clients/bulk-delete", post(clients::bulk_delete))
        .route("/api/v1/clients/bulk-move", post(clients::bulk_move))
        .route(
            "/api/v1/clients/{id}",
            get(clients::get)
                .put(clients::update)
                .delete(clients::delete),
        )
        .route(
            "/api/v1/clients/{id}/tags",
            get(clients::list_tags).post(clients::set_tags),
        )
        .route("/api/v1/images", get(images::list).post(images::create))
        .route(
            "/api/v1/images/files",
            get(images::list_files).post(images::upload_file),
        )
        .route("/api/v1/images/files/meta", get(images::file_meta))
        .route(
            "/api/v1/images/{id}",
            get(images::get).delete(images::delete),
        )
        .route("/api/v1/admins", get(admins::list).post(admins::create))
        .route(
            "/api/v1/admins/{id}",
            put(admins::update).delete(admins::delete),
        )
        .route("/api/v1/edges", get(edges::list).post(edges::create))
        .route(
            "/api/v1/edges/{id}",
            get(edges::get).put(edges::update).delete(edges::delete),
        )
        .route("/api/v1/edges/{id}/replicas", get(edges::list_replicas))
        .route("/api/v1/edges/{id}/sync", post(edges::sync_image))
        .route(
            "/api/v1/edges/{id}/replicas/{image_id}/purge",
            post(edges::purge_replica),
        )
        .route("/api/v1/edges/{id}/purge_all", post(edges::purge_all))
        .route("/api/v1/edges/{id}/rotate_token", post(edges::rotate_token))
        .route("/api/v1/scale/limits", get(edges::scale_limits))
        .route("/api/v1/modes", get(modes::list).post(modes::create))
        .route("/api/v1/modes/bindings", get(modes::list_bindings))
        .route("/api/v1/modes/{id}/schedule", post(modes::update_schedule))
        .route("/api/v1/modes/{id}", delete(modes::delete))
        .route("/api/v1/labs/{id}/mode", post(modes::bind_lab))
        .route("/api/v1/audit", get(audit::list))
        .route("/api/v1/audit/export.csv", get(audit::export_csv))
        .route(
            "/api/v1/ops/class-lock",
            get(ops::get_class_lock).post(ops::set_class_lock),
        )
        .route(
            "/api/v1/ops/enrollment",
            get(ops::get_enrollment).post(ops::set_enrollment),
        )
        .route(
            "/api/v1/ops/task-lifecycle",
            get(ops::get_task_lifecycle).post(ops::set_task_lifecycle),
        )
        .route(
            "/api/v1/ops/power-schedules",
            get(ops::list_schedules).post(ops::create_schedule),
        )
        .route("/api/v1/ops/power-schedules/tick", post(ops::force_tick))
        .route(
            "/api/v1/software/packages",
            get(software::list_packages).post(software::create_package),
        )
        .route(
            "/api/v1/software/packages/{id}/upload",
            put(software::upload_package),
        )
        .route(
            "/api/v1/software/jobs",
            get(software::list_jobs).post(software::create_job),
        )
        .route(
            "/api/v1/software/jobs/{id}/report",
            post(software::report_job),
        )
        .route(
            "/api/v1/software/jobs/{id}/blob",
            get(software::download_for_job),
        )
        .route(
            "/api/v1/assist/sessions",
            get(assist::list).post(assist::create),
        )
        .route("/api/v1/assist/sessions/{id}/ack", post(assist::ack))
        .route("/api/v1/assist/sessions/{id}", get(remote::get))
        .route(
            "/api/v1/assist/sessions/{id}/state",
            get(remote::helper_state),
        )
        .route("/api/v1/assist/sessions/{id}/end", post(remote::end))
        .route(
            "/api/v1/assist/sessions/{id}/frame",
            get(remote::frame_get)
                .put(remote::frame_put)
                .layer(DefaultBodyLimit::max(12 * 1024 * 1024)),
        )
        .route(
            "/api/v1/assist/sessions/{id}/commands/next",
            get(remote::commands_next),
        )
        .route(
            "/api/v1/assist/commands/{id}/report",
            post(remote::command_report),
        )
        .route("/api/v1/assist/sessions/{id}/input", post(remote::input))
        .route(
            "/api/v1/assist/sessions/{id}/clipboard",
            get(remote::clipboard_show)
                .post(remote::clipboard_set)
                .put(remote::clipboard_upload),
        )
        .route(
            "/api/v1/assist/sessions/{id}/clipboard/read",
            post(remote::clipboard_read),
        )
        .route(
            "/api/v1/assist/sessions/{id}/fs/list",
            post(remote::fs_list),
        )
        .route(
            "/api/v1/assist/sessions/{id}/fs/download",
            post(remote::fs_download),
        )
        .route(
            "/api/v1/assist/sessions/{id}/fs/upload",
            post(remote::fs_upload).layer(DefaultBodyLimit::max(512 * 1024 * 1024)),
        )
        .route(
            "/api/v1/assist/fs/{id}",
            get(remote::fs_status).put(remote::fs_result_put),
        )
        .route(
            "/api/v1/assist/fs/{id}/data",
            get(remote::fs_data_get)
                .put(remote::fs_data_put)
                .layer(DefaultBodyLimit::max(512 * 1024 * 1024)),
        )
        .route(
            "/api/v1/command-presets",
            get(command_presets::list).post(command_presets::create),
        )
        .route(
            "/api/v1/command-presets/{id}",
            put(command_presets::update).delete(command_presets::delete),
        )
        .route(
            "/api/v1/tasks/file-collect",
            post(remote::create_file_collect),
        )
        .route(
            "/api/v1/tasks/file-distribute",
            post(remote::create_file_distribute).layer(DefaultBodyLimit::max(512 * 1024 * 1024)),
        )
        .route(
            "/api/v1/file-tasks/{task_id}/bundle",
            get(remote::file_task_bundle),
        )
        .route(
            "/api/v1/file-tasks/{task_id}/{mac}/collect",
            put(remote::collect_upload).layer(DefaultBodyLimit::max(512 * 1024 * 1024)),
        )
        .route(
            "/api/v1/file-tasks/{task_id}/{mac}/result",
            put(remote::file_target_result),
        )
        .route(
            "/api/v1/file-tasks/{task_id}/{mac}/archive",
            get(remote::collect_download),
        )
        .route("/api/v1/agent/heartbeat", post(agent::heartbeat))
        .route("/api/v1/agent/commands/next", get(agent_commands::next))
        .route(
            "/api/v1/agent/commands/{id}/report",
            post(agent_commands::report),
        )
        .route(
            "/api/v1/agent/commands/{id}",
            get(agent_commands::get),
        )
        .route(
            "/api/v1/clients/{mac}/commands",
            post(agent_commands::create),
        )
        .route("/api/v1/agent/restore/report", post(agent::report_restore))
        .route("/api/v1/agent/usb/report", post(agent::report_usb))
        .route("/api/v1/agent/sessions", get(agent::list_sessions))
        .route(
            "/api/v1/images/recommend-delta",
            post(agent::recommend_delta),
        )
        .route(
            "/api/v1/images/build-delta",
            post(delta_images::build_delta_image),
        )
        .route(
            "/api/v1/images/apply-delta",
            post(delta_images::apply_delta_image),
        )
        .route(
            "/api/v1/power/commands",
            get(power::list).post(power::create),
        )
        .route("/api/v1/power/commands/bulk", post(power::create_bulk))
        .route("/api/v1/power/commands/{id}/ack", post(power::ack))
        .route("/api/v1/tasks", get(tasks::list))
        .route("/api/v1/tasks/export.csv", get(tasks::export_csv))
        .route("/api/v1/tasks/capture", post(tasks::create_capture))
        .route("/api/v1/tasks/deploy", post(tasks::create_deploy))
        .route("/api/v1/tasks/deploy/preview", post(tasks::preview_deploy))
        .route("/api/v1/tasks/{id}", get(tasks::get))
        .route("/api/v1/tasks/{id}/cancel", post(tasks::cancel))
        .route(
            "/api/v1/tasks/agent-update",
            post(tasks::create_agent_update),
        )
        .route("/api/v1/agent/packages", get(agent_packages::list))
        .route(
            "/api/v1/agent/packages",
            put(agent_packages::upload).layer(DefaultBodyLimit::max(64 * 1024 * 1024)),
        )
        .route(
            "/api/v1/agent/packages/{version}/download",
            get(agent_packages::download),
        )
        .route("/api/v1/edge/register", post(edge_agent::register))
        .route("/api/v1/edge/heartbeat", post(edge_agent::heartbeat))
        .route("/api/v1/edge/commands/next", get(edge_agent::next_command))
        .route(
            "/api/v1/edge/commands/{id}/report",
            post(edge_agent::report_command),
        )
        .route("/api/v1/edge/images/{id}", get(edge_agent::image_meta))
        .route("/api/v1/edge/images/{id}/blob", get(edge_agent::image_blob))
        .route("/api/v1/edge/tasks/claim", get(edge_agent::claim_task))
        .route("/api/v1/edge/ip-bindings", get(edge_bindings::list))
        .route(
            "/api/v1/edge/tasks/targets/{id}/report",
            post(edge_agent::report_target),
        )
        .route("/api/v1/edge/images/ingest", put(edge_agent::ingest_image))
        .route("/api/v1/pe-sessions", get(pe_sessions::list))
        .route(
            "/api/v1/edge/pe-sessions",
            post(pe_sessions::upsert_from_edge),
        )
}
