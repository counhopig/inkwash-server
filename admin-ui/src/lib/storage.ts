const KEY_ADMIN_TOKEN = "inkpaper.admin.token";

export function loadAdminToken(): string {
  return localStorage.getItem(KEY_ADMIN_TOKEN) ?? "";
}

export function saveAdminToken(value: string): void {
  if (value) localStorage.setItem(KEY_ADMIN_TOKEN, value);
  else localStorage.removeItem(KEY_ADMIN_TOKEN);
}
