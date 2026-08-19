export type Repeat = "Daily" | { Once: { year: number; month: number; day: number } };

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
  month: number;
  day: number;
}

export interface Todo {
  id: number;
  text: string;
  done: boolean;
  importance: Importance;
  due_date: TodoDue | null;
}

export interface Device {
  /** UUID (v4) string, opaque - not a sequential number. */
  id: string;
  name: string;
  /** Only present once, in the response to registerDevice(). */
  token?: string;
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
}
