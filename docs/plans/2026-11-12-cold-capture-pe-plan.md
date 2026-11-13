# Cold Capture Linux PE Implementation Plan

> **For agentic workers:** implement task-by-task.

**Goal:** PXE cold-capture gold-master disk into Center as yixue-image-v1 via streaming pe-agent in Alpine PE.

**Architecture:** Alpine netboot (downloaded) + overlay cpio with musl pe-agent; checkin.ipxe boots PE when capture task claimed; pe-agent streams disk→packed file→HTTP; Edge/Center stream to disk then register Image.

**Tech Stack:** Rust (idv-common/pe-agent/edge/center), Alpine LTS netboot, iPXE, busybox/alpine local.d

## Global Constraints

- Do not load whole disk into RAM
- ASCII-only iPXE echo text
- Filter/exclude files: document only this round
- No git commit unless user asks

---

### Task 1: Streaming image pack + header-from-file

**Files:**
- Modify: `crates/idv-common/src/image_format.rs`

- [ ] Add `pack_image_to_path(header, payload_path, out_path)`
- [ ] Add `inspect_image_file(path) -> ImageHeader` (stream-verify payload hash)
- [ ] Unit test with temp files

### Task 2: Center/Edge streaming ingest

**Files:**
- Modify: `crates/idv-center/src/api/edge_agent.rs` ingest
- Modify: `crates/idv-edge/src/pe_api.rs` capture_upload
- Modify: `crates/idv-edge/src/worker.rs` ingest_capture stream from path

- [ ] Accept body stream → write `{id}.bin` under images_dir
- [ ] Use inspect_image_file; do not `Bytes` whole body

### Task 3: pe-agent physical disk + stream upload

**Files:**
- Modify: `crates/idv-pe-agent/src/main.rs`
- Modify: `deploy/config/pe-agent.toml`

- [ ] `--disk` defaults; Linux `/dev/sda`, Windows `\\.\PhysicalDrive0`
- [ ] Capture: copy disk→temp payload→pack_image_to_path→PUT file stream
- [ ] Parse cmdline/env `IDV_EDGE_URL`

### Task 4: Alpine PE assets + overlay

**Files:**
- Create: `deploy/scripts/fetch-pe-linux.ps1`
- Create: `deploy/scripts/build-pe-overlay.ps1`
- Create: `pe/linux/README.md`
- Output: `pe/linux/vmlinuz-lts`, `initramfs-lts`, `idv.apkovl.tar.gz` or overlay cpio

- [ ] Download Alpine netboot
- [ ] Build musl pe-agent into overlay that auto-runs on boot

### Task 5: iPXE boot PE on capture

**Files:**
- Modify: `crates/idv-edge/src/pe_api.rs` checkin_ipxe
- Modify: `pe/pe.ipxe` if needed
- Modify: `docs/modules/10-采集规范与分区角色.md` filter section

- [ ] has_task capture → kernel/initrd/apkovl URLs on Edge HTTP `/pxe/linux/`
- [ ] Document operator steps: create capture → PXE gold master
