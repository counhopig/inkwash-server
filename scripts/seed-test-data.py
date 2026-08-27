#!/usr/bin/env python3
"""Seed a local inkwash-server SQLite database with bulk alarms / todos /
inbox for the T-056 large-data hardware verification (pagination,
truncation, NVS capacity, e-paper ghosting).

Usage:
  seed-test-data.py <db> <device_id> [--alarms N] [--todos N] [--inbox N]

Defaults are sized to fill the device-side caps: the alarms NVS blob is
~1024B (about 12 alarms with labels), todos ~2048B (about 22), and the
server sends at most 20 inbox items per sync (INBOX_LIMIT), so --inbox 40
forces `inbox_truncated=true` and the "more on server" hint.

Re-runnable: wipes the target device's alarms/todos/inbox, then inserts
fresh rows. Inbox items are seeded unread with priority "normal" so the
large-data pass does not trigger urgent full-screen reminders (which would
interrupt the display checks); todos use medium/low importance and no
today-due dates for the same reason.
"""
import argparse
import sqlite3
import sys
import time
import uuid

REPEAT_KINDS = ("daily", "once", "weekly", "monthly")
IMPORTANCES = ("medium", "low")


def now() -> int:
    return int(time.time())


def seed(db_path: str, device_id: str, alarms_n: int, todos_n: int, inbox_n: int) -> None:
    con = sqlite3.connect(db_path)
    con.execute("PRAGMA foreign_keys = ON")
    try:
        cur = con.cursor()

        # Verify the device exists so the FK constraints below cannot silently
        # no-op the whole seed.
        dev = cur.execute(
            "SELECT id FROM devices WHERE id = ?", (device_id,)
        ).fetchone()
        if dev is None:
            raise SystemExit(f"device {device_id!r} not found in {db_path}")

        # --- alarms ---------------------------------------------------------
        cur.execute("DELETE FROM alarms WHERE device_id = ?", (device_id,))
        for i in range(alarms_n):
            kind = REPEAT_KINDS[i % len(REPEAT_KINDS)]
            once = (2026, 12, 1 + (i % 27)) if kind == "once" else (None, None, None)
            days = "[0,2,4]" if kind == "weekly" else ("[1]" if kind == "monthly" else None)
            cur.execute(
                """INSERT INTO alarms
                   (device_id, local_id, hour, minute, repeat_kind,
                    once_year, once_month, once_day, repeat_days, enabled, label)
                   VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)""",
                (
                    device_id,
                    i,
                    6 + (i * 5) % 17,  # 06:00..22:00 so the seeded alarms never ring at night
                    (i * 13) % 60,
                    kind,
                    once[0],
                    once[1],
                    once[2],
                    days,
                    1,
                    f"闹钟测试 {i:02d} 买菜浇水",
                ),
            )

        # --- todos ----------------------------------------------------------
        cur.execute("DELETE FROM todos WHERE device_id = ?", (device_id,))
        for i in range(todos_n):
            repeat_kind = "weekly" if i % 5 == 0 else None
            cur.execute(
                """INSERT INTO todos
                   (device_id, local_id, text, done, importance,
                    due_year, due_month, due_day, repeat_kind, repeat_days)
                   VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)""",
                (
                    device_id,
                    i,
                    f"Todo {i:02d} 完成周报并回复邮件 - extra text to fill the NVS blob",
                    0,
                    IMPORTANCES[i % len(IMPORTANCES)],
                    None,
                    None,
                    None,
                    repeat_kind,
                    "[1,3,5]" if repeat_kind else None,
                ),
            )

        # --- inbox ----------------------------------------------------------
        cur.execute("DELETE FROM inbox WHERE device_id = ?", (device_id,))
        cur.execute(
            """INSERT OR IGNORE INTO channels
               (id, device_id, kind, name, enabled, config_version,
                created_at, updated_at)
               VALUES (?, ?, 'webhook', 'seed-bulk', 1, 1, ?, ?)""",
            ("seed-channel", device_id, now(), now()),
        )
        cur.execute(
            """INSERT INTO device_sequences (device_id, next_inbox_seq)
               VALUES (?, ?)
               ON CONFLICT(device_id) DO UPDATE SET next_inbox_seq = excluded.next_inbox_seq""",
            (device_id, inbox_n + 1),
        )
        for i in range(inbox_n):
            cur.execute(
                """INSERT INTO inbox
                   (id, device_id, channel_id, seq, kind, priority,
                    title, body, when_epoch, source_ref, read,
                    created_at, updated_at)
                   VALUES (?, ?, 'seed-channel', ?, 'alert', 'normal', ?, ?, ?, ?, 0, ?, ?)""",
                (
                    str(uuid.uuid4()),
                    device_id,
                    i + 1,
                    f"Inbox {i:02d} 服务器通知标题",
                    f"正文内容第 {i:02d} 条，用于观察列表分页与 e-paper 重影。",
                    now() - (inbox_n - i) * 60,
                    f"seed-{i}",
                    now(),
                    now(),
                ),
            )

        con.commit()
    finally:
        con.close()


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("db", help="path to the local SQLite database")
    ap.add_argument("device_id")
    ap.add_argument("--alarms", type=int, default=12, help="default 12 (~1024B NVS blob)")
    ap.add_argument("--todos", type=int, default=22, help="default 22 (~2048B NVS blob)")
    ap.add_argument("--inbox", type=int, default=40, help="default 40 (20 delivered + truncation)")
    args = ap.parse_args()
    seed(args.db, args.device_id, args.alarms, args.todos, args.inbox)
    print(
        f"seeded {args.alarms} alarms, {args.todos} todos, {args.inbox} inbox "
        f"for device {args.device_id} in {args.db}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
