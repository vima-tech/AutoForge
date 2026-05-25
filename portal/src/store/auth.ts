const KEY_ADMIN_ID = "autoforge_admin_id";
const KEY_ADMIN_KEY = "autoforge_admin_key";

export function getAdminHeaders(): Record<string, string> {
  const id = localStorage.getItem(KEY_ADMIN_ID) || "admin";
  const key = localStorage.getItem(KEY_ADMIN_KEY) || import.meta.env.VITE_ADMIN_API_KEY || "";
  return { "x-admin-id": id, "x-admin-key": key };
}

export function setAdminCredentials(id: string, key: string) {
  localStorage.setItem(KEY_ADMIN_ID, id);
  localStorage.setItem(KEY_ADMIN_KEY, key);
}

export function getAdminId(): string {
  return localStorage.getItem(KEY_ADMIN_ID) || "admin";
}

export function isConfigured(): boolean {
  const key = localStorage.getItem(KEY_ADMIN_KEY) || import.meta.env.VITE_ADMIN_API_KEY || "";
  return key.length > 0;
}
