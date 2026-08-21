// Thin fetch wrapper for the admin API (see ../../../src/routes.rs). This
// page is served by inkpaper-server itself at "/", so every call is
// same-origin - no base URL to configure. Auth is a bearer token: either a
// console-account session or the admin token.

import type {
  AccountSummary,
  Alarm,
  Channel,
  ChannelCreated,
  Device,
  InboxItem,
  Todo,
  UpsertAlarmInput,
  UpsertTodoInput,
} from "./types";

export interface AuthResponse {
  token: string;
  username: string;
}

export type Result<T> =
  | { ok: true; value: T }
  | { ok: false; status: number; error: string };

async function request<T>(path: string, token: string, init: RequestInit = {}): Promise<Result<T>> {
  try {
    const res = await fetch(path, {
      ...init,
      headers: {
        ...(token ? { Authorization: `Bearer ${token}` } : {}),
        ...(init.body ? { "Content-Type": "application/json" } : {}),
        ...init.headers,
      },
    });
    if (!res.ok) {
      const text = await res.text().catch(() => "");
      return { ok: false, status: res.status, error: text || `HTTP ${res.status}` };
    }
    if (res.status === 204) return { ok: true, value: undefined as T };
    return { ok: true, value: (await res.json()) as T };
  } catch (e) {
    return { ok: false, status: 0, error: e instanceof Error ? e.message : String(e) };
  }
}

// --- Auth (console accounts) ---------------------------------------------

export function registerAccount(username: string, password: string) {
  return request<AuthResponse>("/api/auth/register", "", {
    method: "POST",
    body: JSON.stringify({ username, password }),
  });
}

export function loginAccount(username: string, password: string) {
  return request<AuthResponse>("/api/auth/login", "", {
    method: "POST",
    body: JSON.stringify({ username, password }),
  });
}

export function logoutAccount(token: string) {
  return request<void>("/api/auth/logout", token, { method: "POST" });
}

/** Validates a stored token on load; returns `{kind:"admin"}` or an account
 *  payload, or 401 if the session is no longer valid. */
export function me(token: string) {
  return request<{ kind: "admin" } | { kind: "account"; account_id: number; username: string }>(
    "/api/auth/me",
    token,
  );
}

export function changePassword(token: string, oldPassword: string, newPassword: string) {
  return request<void>("/api/auth/password", token, {
    method: "POST",
    body: JSON.stringify({ old_password: oldPassword, new_password: newPassword }),
  });
}

// --- Admin: account management (ADMIN_TOKEN only) --------------------------

export function listAccounts(token: string) {
  return request<AccountSummary[]>("/api/admin/accounts", token);
}

export function deleteAccount(token: string, accountId: number) {
  return request<void>(`/api/admin/accounts/${accountId}`, token, { method: "DELETE" });
}

export function resetAccountPassword(token: string, accountId: number, newPassword: string) {
  return request<void>(`/api/admin/accounts/${accountId}/password`, token, {
    method: "POST",
    body: JSON.stringify({ new_password: newPassword }),
  });
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

export function deleteDevice(token: string, id: string) {
  return request<void>(`/api/devices/${id}`, token, { method: "DELETE" });
}

export function listAlarms(token: string, deviceId: string) {
  return request<Alarm[]>(`/api/devices/${deviceId}/alarms`, token);
}

export function createAlarm(token: string, deviceId: string, input: UpsertAlarmInput) {
  return request<{ id: number }>(`/api/devices/${deviceId}/alarms`, token, {
    method: "POST",
    body: JSON.stringify(input),
  });
}

export function deleteAlarm(token: string, deviceId: string, alarmId: number) {
  return request<void>(`/api/devices/${deviceId}/alarms/${alarmId}`, token, { method: "DELETE" });
}

export function updateAlarm(token: string, deviceId: string, alarmId: number, input: UpsertAlarmInput) {
  return request<void>(`/api/devices/${deviceId}/alarms/${alarmId}`, token, {
    method: "PUT",
    body: JSON.stringify(input),
  });
}

export function clearAlarms(token: string, deviceId: string) {
  return request<void>(`/api/devices/${deviceId}/alarms`, token, { method: "DELETE" });
}

export function listTodos(token: string, deviceId: string) {
  return request<Todo[]>(`/api/devices/${deviceId}/todos`, token);
}

export function createTodo(token: string, deviceId: string, input: UpsertTodoInput) {
  return request<{ id: number }>(`/api/devices/${deviceId}/todos`, token, {
    method: "POST",
    body: JSON.stringify(input),
  });
}

export function updateTodo(token: string, deviceId: string, todoId: number, input: UpsertTodoInput) {
  return request<void>(`/api/devices/${deviceId}/todos/${todoId}`, token, {
    method: "PUT",
    body: JSON.stringify(input),
  });
}

export function deleteTodo(token: string, deviceId: string, todoId: number) {
  return request<void>(`/api/devices/${deviceId}/todos/${todoId}`, token, { method: "DELETE" });
}

export function clearTodos(token: string, deviceId: string) {
  return request<void>(`/api/devices/${deviceId}/todos`, token, { method: "DELETE" });
}

// --- Channels & inbox ------------------------------------------------------

export function listChannels(token: string, deviceId: string) {
  return request<Channel[]>(`/api/devices/${deviceId}/channels`, token);
}

export function createChannel(token: string, deviceId: string, kind: string, name: string) {
  return request<ChannelCreated>(`/api/devices/${deviceId}/channels`, token, {
    method: "POST",
    body: JSON.stringify({ kind, name }),
  });
}

export function deleteChannel(token: string, deviceId: string, channelId: string) {
  return request<void>(`/api/devices/${deviceId}/channels/${channelId}`, token, {
    method: "DELETE",
  });
}

export function rotateChannelToken(token: string, deviceId: string, channelId: string) {
  return request<{ token: string; token_prefix: string }>(
    `/api/devices/${deviceId}/channels/${channelId}/rotate-token`,
    token,
    { method: "POST" },
  );
}

export function listInbox(token: string, deviceId: string) {
  return request<InboxItem[]>(`/api/devices/${deviceId}/inbox`, token);
}

export function deleteInboxItem(token: string, deviceId: string, seq: number) {
  return request<void>(`/api/devices/${deviceId}/inbox/${seq}`, token, { method: "DELETE" });
}

export function clearInbox(token: string, deviceId: string) {
  return request<void>(`/api/devices/${deviceId}/inbox`, token, { method: "DELETE" });
}
