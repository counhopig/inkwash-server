export type Repeat = "Daily" | { Once: { year: number; month: number; day: number } };

export interface Alarm {
  id: number;
  hour: number;
  minute: number;
  repeat: Repeat;
  enabled: boolean;
  label: string;
}

export interface Todo {
  id: number;
  text: string;
  done: boolean;
}

export interface Device {
  id: number;
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
}
