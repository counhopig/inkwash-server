// Shared console UI services, provided by App.vue and consumed by the view
// components (Dashboard/Device/Account). Kept dependency-free: no router, no
// state library - views switch via a simple ref in App.vue.

import type { InjectionKey, Ref } from "vue";
import type { Result } from "./api";
import type { Session } from "./storage";

export interface UI {
  session: Readonly<Ref<Session | null>>;
  authToken(): string;
  toast(text: string, variant?: "info" | "error"): void;
  confirmDialog(text: string): Promise<boolean>;
  /** Prompts for a single value; resolves the trimmed input, or null when
   *  cancelled. */
  promptDialog(message: string, placeholder?: string): Promise<string | null>;
  run<T>(name: string, fn: () => Promise<Result<T>>): Promise<T | null>;
  isBusy(name: string): boolean;
  onUnauthorized(): void;
}

export const uiKey: InjectionKey<UI> = Symbol("inkwash-ui");
