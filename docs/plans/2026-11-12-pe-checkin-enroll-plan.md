# PE Check-in Enroll Implementation Plan

> **For agentic workers:** implement task-by-task; verify each step.

**Goal:** Unknown MAC at PE check-in is auto-enrolled with lab naming; iPXE shows the name.

**Files:**
- `crates/idv-center/src/enroll.rs` (new) — shared ensure_client_enrolled
- `crates/idv-center/src/api/agent.rs` — use enroll helper
- `crates/idv-center/src/api/pe_sessions.rs` — enroll on upsert, JSON response
- `crates/idv-center/src/lib` or `main` / `api/mod.rs` — mod enroll
- `crates/idv-edge/src/worker.rs` — parse enroll response
- `crates/idv-edge/src/pe_api.rs` — CheckinResponse fields + checkin.ipxe
- `crates/idv-edge/src/main.rs` — route checkin.ipxe
- `pe/pe.ipxe` — chain checkin.ipxe

## Tasks

1. Add `enroll.rs` + wire agent heartbeat
2. pe_sessions upsert returns enroll JSON
3. Edge checkin + checkin.ipxe + pe.ipxe
4. Restart center/edge; PXE once and confirm Clients page
