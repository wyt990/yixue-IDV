use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Admin {
    pub id: String,
    pub username: String,
    #[serde(skip_serializing)]
    pub password_hash: String,
    pub display_name: String,
    /// super | lab_admin | readonly
    #[serde(default = "default_admin_role")]
    pub role: String,
    pub created_at: DateTime<Utc>,
}

fn default_admin_role() -> String {
    "super".into()
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Campus {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub default_image_id: Option<String>,
    pub created_at: DateTime<Utc>,
    /// Bound Edge ids ordered by priority (not a DB column).
    #[sqlx(skip)]
    #[serde(default)]
    pub edge_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Lab {
    pub id: String,
    pub campus_id: String,
    pub name: String,
    #[serde(default)]
    pub name_prefix: String,
    #[serde(default = "default_suffix_width")]
    pub suffix_width: i64,
    #[serde(default = "default_suffix_start")]
    pub suffix_start: i64,
    #[serde(default = "default_suffix_start")]
    pub next_suffix: i64,
    #[serde(default)]
    pub ip_start: String,
    #[serde(default)]
    pub ip_end: String,
    #[serde(default)]
    pub default_image_id: Option<String>,
    pub created_at: DateTime<Utc>,
    /// Bound Edge ids ordered by priority (not a DB column).
    #[sqlx(skip)]
    #[serde(default)]
    pub edge_ids: Vec<String>,
}

fn default_suffix_width() -> i64 {
    3
}

fn default_suffix_start() -> i64 {
    1
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Client {
    pub id: String,
    pub lab_id: String,
    pub mac: String,
    pub name: String,
    pub notes: String,
    pub last_seen: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Image {
    pub id: String,
    pub name: String,
    pub os_family: String,
    pub version_label: String,
    pub checksum_sha256: String,
    pub size_bytes: i64,
    pub storage_uri: String,
    pub boot_mode: String,
    pub notes: String,
    pub partition_map: String,
    pub content_type: String,
    pub base_image_id: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub token: String,
    pub username: String,
    pub display_name: String,
    pub role: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateCampus {
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateCampus {
    pub name: String,
    /// When `edge_ids` is `Some`, also apply this (empty/None clears).
    #[serde(default)]
    pub default_image_id: Option<String>,
    /// When `Some`, replace campus Edge bindings (order = priority).
    #[serde(default)]
    pub edge_ids: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct CreateLab {
    pub campus_id: String,
    pub name: String,
    #[serde(default)]
    pub name_prefix: String,
    #[serde(default = "default_suffix_width")]
    pub suffix_width: i64,
    #[serde(default = "default_suffix_start")]
    pub suffix_start: i64,
    #[serde(default)]
    pub ip_start: String,
    #[serde(default)]
    pub ip_end: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateLab {
    #[serde(default)]
    pub campus_id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub name_prefix: Option<String>,
    #[serde(default)]
    pub suffix_width: Option<i64>,
    #[serde(default)]
    pub suffix_start: Option<i64>,
    #[serde(default)]
    pub next_suffix: Option<i64>,
    #[serde(default)]
    pub ip_start: Option<String>,
    #[serde(default)]
    pub ip_end: Option<String>,
    /// When `edge_ids` is `Some`, also apply this (empty/None clears).
    #[serde(default)]
    pub default_image_id: Option<String>,
    /// When `Some`, replace lab Edge bindings (order = priority).
    #[serde(default)]
    pub edge_ids: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EnrollmentSettings {
    #[serde(default)]
    pub default_campus_id: String,
    #[serde(default)]
    pub default_lab_id: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateClient {
    pub lab_id: String,
    pub mac: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub notes: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateClient {
    #[serde(default)]
    pub lab_id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateImage {
    pub name: String,
    pub os_family: String,
    #[serde(default)]
    pub version_label: String,
    pub checksum_sha256: String,
    #[serde(default)]
    pub size_bytes: i64,
    #[serde(default)]
    pub storage_uri: String,
    #[serde(default = "default_boot_mode")]
    pub boot_mode: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default = "default_partition_map")]
    pub partition_map: String,
    #[serde(default = "default_content_type")]
    pub content_type: String,
    #[serde(default)]
    pub base_image_id: Option<String>,
}

fn default_content_type() -> String {
    "full".into()
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct ModePolicy {
    pub id: String,
    pub name: String,
    pub restore: String,
    pub system_action: String,
    pub persistent_action: String,
    pub volatile_action: String,
    pub notes: String,
    pub schedule_start: Option<String>,
    pub schedule_end: Option<String>,
    pub usb_restricted: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateModePolicy {
    pub name: String,
    #[serde(default = "default_restore")]
    pub restore: String,
    #[serde(default = "default_system_action")]
    pub system_action: String,
    #[serde(default = "default_persistent_action")]
    pub persistent_action: String,
    #[serde(default = "default_volatile_action")]
    pub volatile_action: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub schedule_start: Option<String>,
    #[serde(default)]
    pub schedule_end: Option<String>,
    #[serde(default)]
    pub usb_restricted: bool,
}

#[derive(Debug, Deserialize)]
pub struct UpdateModeSchedule {
    #[serde(default)]
    pub schedule_start: Option<String>,
    #[serde(default)]
    pub schedule_end: Option<String>,
    #[serde(default)]
    pub usb_restricted: Option<bool>,
}

fn default_restore() -> String {
    "reboot".into()
}
fn default_system_action() -> String {
    "restore".into()
}
fn default_persistent_action() -> String {
    "keep".into()
}
fn default_volatile_action() -> String {
    "wipe".into()
}

#[derive(Debug, Deserialize)]
pub struct BindLabMode {
    pub mode_policy_id: String,
    #[serde(default = "default_effective")]
    pub effective: String,
}

fn default_effective() -> String {
    "next_boot".into()
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct LabModeBinding {
    pub lab_id: String,
    pub mode_policy_id: String,
    pub effective: String,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct AgentHeartbeat {
    pub mac: String,
    #[serde(default)]
    pub hostname: String,
    #[serde(default)]
    pub agent_version: String,
    #[serde(default = "default_trigger")]
    pub trigger: String,
}

fn default_trigger() -> String {
    "heartbeat".into()
}

#[derive(Debug, Serialize)]
pub struct AgentPolicyResponse {
    pub mac: String,
    pub client_id: Option<String>,
    pub lab_id: Option<String>,
    pub mode: Option<ModePolicy>,
    pub baseline_image_id: Option<String>,
    pub should_restore: bool,
    pub schedule_active: bool,
    pub usb_restricted: bool,
    pub pending_power: Option<PowerCommand>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_software: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_assist: Option<serde_json::Value>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct PowerCommand {
    pub id: String,
    pub client_id: Option<String>,
    pub mac: String,
    pub kind: String,
    pub state: String,
    pub detail: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreatePowerCommand {
    pub mac: String,
    pub kind: String,
    #[serde(default)]
    pub detail: String,
}

#[derive(Debug, Deserialize)]
pub struct BuildDeltaRequest {
    pub base_image_id: String,
    pub target_image_id: String,
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct ApplyDeltaRequest {
    pub base_image_id: String,
    pub delta_image_id: String,
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct AgentRestoreReport {
    pub mac: String,
    pub ok: bool,
    #[serde(default)]
    pub detail: String,
}

#[derive(Debug, Deserialize)]
pub struct AgentUsbReport {
    pub mac: String,
    pub enforced: bool,
    #[serde(default)]
    pub detail: String,
}

#[derive(Debug, Deserialize)]
pub struct DeltaRecommendRequest {
    pub base_image_id: String,
    pub target_image_id: String,
    #[serde(default)]
    pub force_full: bool,
    /// Estimated delta size ratio vs full (0.0–1.0). Optional; if omitted uses size_bytes ratio heuristic.
    #[serde(default)]
    pub estimated_delta_ratio: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct DeltaRecommendResponse {
    pub recommendation: String,
    pub reason: String,
    pub content_type: String,
}

fn default_partition_map() -> String {
    "[]".into()
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct MaintenanceTask {
    pub id: String,
    pub kind: String,
    pub state: String,
    pub image_id: Option<String>,
    pub edge_id: Option<String>,
    pub disk_plan: String,
    pub name: String,
    pub error_message: String,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct MaintenanceTaskTarget {
    pub id: String,
    pub task_id: String,
    pub mac: String,
    pub state: String,
    pub progress: f64,
    pub error_message: String,
    pub detail: String,
    pub p2p_bytes: i64,
    pub edge_bytes: i64,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateCaptureTask {
    pub mac: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub edge_id: Option<String>,
    #[serde(default = "default_os_family")]
    pub os_family: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateDeployTask {
    pub image_id: String,
    pub macs: Vec<String>,
    #[serde(default = "default_disk_plan")]
    pub disk_plan: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub edge_id: Option<String>,
    /// UI must send true after showing destructive confirm.
    #[serde(default)]
    pub confirm_destructive: bool,
    /// Bypass class-hour deploy lock (super only, audited).
    #[serde(default)]
    pub force_class_lock: bool,
}

fn default_os_family() -> String {
    "windows".into()
}
fn default_disk_plan() -> String {
    "wipe_whole_disk".into()
}

fn default_boot_mode() -> String {
    "uefi".into()
}

#[derive(Debug, Deserialize)]
pub struct CreateAdmin {
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default = "default_admin_role")]
    pub role: String,
}

#[derive(Debug, Serialize)]
pub struct DashboardStats {
    pub campuses: i64,
    pub labs: i64,
    pub clients: i64,
    pub images: i64,
    pub edges_online: i64,
    pub edges_total: i64,
    pub tasks_running: i64,
}

#[derive(Debug, Serialize)]
pub struct DashboardEdgeRow {
    pub id: String,
    pub name: String,
    pub online: bool,
    pub cpu_pct: f64,
    pub mem_used_bytes: i64,
    pub mem_total_bytes: i64,
    pub net_rx_bps: i64,
    pub net_tx_bps: i64,
    pub last_seen: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
pub struct DashboardTaskRow {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub state: String,
    pub progress_text: String,
    pub error_message: String,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct DashboardLabRow {
    pub id: String,
    pub name: String,
    pub campus_name: String,
    pub client_count: i64,
    pub default_image_name: String,
    pub image_from_campus: bool,
}

#[derive(Debug, Serialize)]
pub struct DashboardOverview {
    pub stats: DashboardStats,
    pub edges: Vec<DashboardEdgeRow>,
    pub tasks_active: Vec<DashboardTaskRow>,
    pub tasks_failed: Vec<DashboardTaskRow>,
    pub labs: Vec<DashboardLabRow>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Edge {
    pub id: String,
    pub name: String,
    pub node_token: String,
    pub listen_url: String,
    pub last_seen: Option<DateTime<Utc>>,
    pub cache_used_bytes: i64,
    pub cache_quota_bytes: i64,
    pub cache_warn_pct: f64,
    pub cache_level: String,
    /// Absolute/relative path on the Edge host for PE capture staging. Empty = Edge default.
    #[serde(default)]
    pub capture_tmp_dir: String,
    #[serde(default)]
    pub cpu_pct: f64,
    #[serde(default)]
    pub mem_used_bytes: i64,
    #[serde(default)]
    pub mem_total_bytes: i64,
    #[serde(default)]
    pub net_rx_bps: i64,
    #[serde(default)]
    pub net_tx_bps: i64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct ImageReplica {
    pub id: String,
    pub edge_id: String,
    pub image_id: String,
    pub state: String,
    pub progress: f64,
    pub local_path: String,
    pub checksum_sha256: String,
    pub error_message: String,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct EdgeCommand {
    pub id: String,
    pub edge_id: String,
    pub kind: String,
    pub image_id: Option<String>,
    pub state: String,
    pub progress: f64,
    pub error_message: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateEdge {
    pub name: String,
    #[serde(default)]
    pub node_token: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateEdge {
    pub name: String,
    /// PE capture staging directory on the Edge machine. Empty clears to Edge local default.
    #[serde(default)]
    pub capture_tmp_dir: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SyncImageRequest {
    pub image_id: String,
    #[serde(default)]
    pub purge_first: bool,
}

#[derive(Debug, Deserialize)]
pub struct EdgeRegisterRequest {
    pub name: String,
    #[serde(default)]
    pub listen_url: String,
}

#[derive(Debug, Default, Deserialize)]
pub struct EdgeHeartbeatRequest {
    #[serde(default)]
    pub cache_used_bytes: i64,
    #[serde(default)]
    pub cache_quota_bytes: i64,
    #[serde(default = "default_cache_warn")]
    pub cache_warn_pct: f64,
    #[serde(default)]
    pub cpu_pct: f64,
    #[serde(default)]
    pub mem_used_bytes: i64,
    #[serde(default)]
    pub mem_total_bytes: i64,
    #[serde(default)]
    pub net_rx_bps: i64,
    #[serde(default)]
    pub net_tx_bps: i64,
}

fn default_cache_warn() -> f64 {
    85.0
}

#[derive(Debug, Serialize)]
pub struct ScaleLimits {
    pub max_concurrent_deploy_targets: i64,
    pub max_pending_sync_per_edge: i64,
    pub recommended_deploy_batch: i64,
}

#[derive(Debug, Deserialize)]
pub struct EdgeCommandReport {
    pub state: String,
    #[serde(default)]
    pub progress: f64,
    #[serde(default)]
    pub error_message: String,
    #[serde(default)]
    pub local_path: String,
    #[serde(default)]
    pub checksum_sha256: String,
}

#[derive(Debug, Serialize)]
pub struct EdgeView {
    #[serde(flatten)]
    pub edge: Edge,
    pub online: bool,
}

pub fn new_id() -> String {
    Uuid::new_v4().to_string()
}

pub fn edge_online(last_seen: Option<DateTime<Utc>>) -> bool {
    last_seen
        .map(|ts| Utc::now().signed_duration_since(ts).num_seconds() <= 20)
        .unwrap_or(false)
}
