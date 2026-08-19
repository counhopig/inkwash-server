// Thin fetch wrapper for the admin API (see ../../../src/routes.rs). This
// page is served by inkpaper-server itself at "/", so every call is
// same-origin - no base URL to configure, just the admin bearer token.

import type { Alarm, Device, Todo, UpsertAlarmInput, UpsertTodoInput } from "./types";

export type Result<T> = { ok: true; value: T } | { ok: false; error: string };

async function request<T>(path: string, token: string, init: RequestInit = {}): Promise<Result<T>> {
  try {
    const res = await fetch(path, {
      ...init,
      headers: {
        Authorization: `Bearer ${token}`,
        ...(init.body ? { "Content-Type": "application/json" } : {}),
        ...init.headers,
      },
    });
    if (!res.ok) {
      const text = await res.text().catch(() => "");
      return { ok: false, error: `HTTP ${res.status}${text ? `: ${text}` : ""}` };
    }
    if (res.status === 204) return { ok: true, value: undefined as T };
    return { ok: true, value: (await res.json()) as T };
  } catch (e) {
    return { ok: false, error: e instanceof Error ? e.message : String(e) };
  }
}

export function listDevices(token: string) {
  return request<Device[]>("/api/devices", token);
}

export function registerDevice(token: string, name: string) {
  return request<Device>("/api/devices", token, {
    method: "POST",
    body: JSON.stringify({ name }),
  });
}

export function deleteDevice(token: string, id: number) {
  return request<void>(`/api/devices/${id}`, token, { method: "DELETE" });
}

export function listAlarms(token: string, deviceId: number) {
  return request<Alarm[]>(`/api/devices/${deviceId}/alarms`, token);
}

export function createAlarm(token: string, deviceId: number, input: UpsertAlarmInput) {
  return request<{ id: number }>(`/api/devices/${deviceId}/alarms`, token, {
    method: "POST",
    body: JSON.stringify(input),
  });
}

export function deleteAlarm(token: string, deviceId: number, alarmId: number) {
  return request<void>(`/api/devices/${deviceId}/alarms/${alarmId}`, token, { method: "DELETE" });
}

export function clearAlarms(token: string, deviceId: number) {
  return request<void>(`/api/devices/${deviceId}/alarms`, token, { method: "DELETE" });
}

export function listTodos(token: string, deviceId: number) {
  return request<Todo[]>(`/api/devices/${deviceId}/todos`, token);
}

export function createTodo(token: string, deviceId: number, input: UpsertTodoInput) {
  return request<{ id: number }>(`/api/devices/${deviceId}/todos`, token, {
    method: "POST",
    body: JSON.stringify(input),
  });
}

export function updateTodo(token: string, deviceId: number, todoId: number, input: UpsertTodoInput) {
  return request<void>(`/api/devices/${deviceId}/todos/${todoId}`, token, {
    method: "PUT",
    body: JSON.stringify(input),
  });
}

export function deleteTodo(token: string, deviceId: number, todoId: number) {
  return request<void>(`/api/devices/${deviceId}/todos/${todoId}`, token, { method: "DELETE" });
}

export function clearTodos(token: string, deviceId: number) {
  return request<void>(`/api/devices/${deviceId}/todos`, token, { method: "DELETE" });
}
