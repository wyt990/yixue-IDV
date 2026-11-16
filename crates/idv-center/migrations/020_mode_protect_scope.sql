-- 2026-11-16: per-mode restore protection scope.
-- system     = only the Windows system volume (exam / manual)
-- all_fixed  = every fixed (non-removable) NTFS volume, e.g. C/D/E/F/G (teaching / boot-restore)
ALTER TABLE mode_policies ADD COLUMN protect_scope TEXT NOT NULL DEFAULT 'system';
UPDATE mode_policies SET protect_scope = 'all_fixed' WHERE name = 'teaching';
UPDATE mode_policies SET notes = '教学还原：开机还原所有固定盘（C/D/E…），含数据盘'
    WHERE name = 'teaching' AND notes = '教学还原：开机还原系统区，保留数据区';
-- Bump bound labs' revision so Agents re-apply the new protect scope on upgrade.
UPDATE lab_mode_bindings
   SET updated_at = strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')
 WHERE mode_policy_id IN (SELECT id FROM mode_policies WHERE protect_scope = 'all_fixed');
