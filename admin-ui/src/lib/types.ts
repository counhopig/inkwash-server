/** Weekdays are 0=Sunday..6=Saturday; month days are 1..=31. */
export type Repeat =
  | "Daily"
  | { Weekly: { days: number[] } }
  | { Monthly: { days: number[] } }
  | { Once: { year: number; month: number; day: number } };

export interface Alarm {
  id: number;
  hour: number;
  minute: number;
  repeat: Repeat;
  enabled: boolean;
  label: string;
}

export type Importance = "low" | "medium" | "high";

export interface TodoDue {
  year: number;
  month: number;
  day: number;
}

export interface Todo {
  id: number;
  text: string;
  done: boolean;
  importance: Importance;
  due_date: TodoDue | null;
  repeat: Repeat | null;
}

export interface Device {
  /** UUID (v4) string, opaque - not a sequential number. */
  id: string;
  name: string;
  /** Only present once, in the response to registerDevice(). */
  token?: string;
}

/** Admin-only view of a console account (no password hash is exposed). */
export interface AccountSummary {
  id: number;
  username: string;
  created_at: number;
  device_count: number;
  session_count: number;
}

export interface UpsertAlarmInput {
  hour: number;
  minute: number;
  repeat: Repeat;
  enabled: boolean;
  label: string;
}

export interface UpsertTodoInput {
  text: string;
  done: boolean;
  importance: Importance;
  due_date: TodoDue | null;
  repeat: Repeat | null;
}

export type ChannelKind = "webhook" | "caldav_basic";

export interface Channel {
  id: string;
  device_id: string;
  kind: ChannelKind;
  name: string;
  enabled: boolean;
  token_prefix: string;
  last_sync_at: number | null;
  last_sync_error: string | null;
  created_at: number;
  updated_at: number;
}

export interface ChannelCreated {
  channel: Channel;
  token?: string;
  delivery_url?: string;
}

export type InboxKind = "alert" | "event" | "info";
export type InboxPriority = "normal" | "high";

export interface InboxItem {
  id: number;
  kind: InboxKind;
  priority: InboxPriority;
  title: string;
  body: string;
  when: number | null;
  read: boolean;
}
