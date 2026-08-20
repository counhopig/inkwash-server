const KEY_ADMIN_TOKEN = "inkpaper.admin.token";
const KEY_SESSION = "inkpaper.admin.session";

export function loadAdminToken(): string {
  return localStorage.getItem(KEY_ADMIN_TOKEN) ?? "";
}

export function saveAdminToken(value: string): void {
  if (value) localStorage.setItem(KEY_ADMIN_TOKEN, value);
  else localStorage.removeItem(KEY_ADMIN_TOKEN);
}

/** A console session (account login). `kind` distinguishes the plain admin
 *  token from a real account session so the UI can show different controls. */
export interface Session {
  kind: "account" | "admin";
  token: string;
  username: string;
}

export function loadSession(): Session | null {
  const raw = localStorage.getItem(KEY_SESSION);
  if (!raw) return null;
  try {
    return JSON.parse(raw) as Session;
  } catch {
    localStorage.removeItem(KEY_SESSION);
    return null;
  }
}

export function saveSession(session: Session | null): void {
  if (session) localStorage.setItem(KEY_SESSION, JSON.stringify(session));
  else localStorage.removeItem(KEY_SESSION);
}
