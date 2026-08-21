<script setup lang="ts">
import { computed, onMounted, provide, ref } from "vue";
import * as api from "./lib/api";
import { loadSession, saveSession, type Session } from "./lib/storage";
import { uiKey, type UI } from "./lib/ui";
import type { Device } from "./lib/types";
import LoginView from "./LoginView.vue";
import DashboardView from "./DashboardView.vue";
import DeviceView from "./DeviceView.vue";
import AccountView from "./AccountView.vue";

const session = ref<Session | null>(null);
const checkingSession = ref(true);
const busy = ref<string | null>(null);

const view = ref<"dashboard" | "device" | "account">("dashboard");
const devices = ref<Device[]>([]);
const stats = ref<Record<string, { alarms: number; todos: number; done: number }>>({});

const selectedDevice = computed(
  () => devices.value.find((d) => d.id === selectedDeviceId.value) ?? null,
);
const selectedDeviceId = ref<string | null>(null);

// --- Toast ---------------------------------------------------------------

const toastText = ref("");
const toastVariant = ref<"info" | "error">("info");
const toastVisible = ref(false);
let toastTimer: ReturnType<typeof setTimeout> | undefined;

function toast(text: string, variant: "info" | "error" = "info") {
  toastText.value = text;
  toastVariant.value = variant;
  toastVisible.value = true;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (toastVisible.value = false), variant === "error" ? 4200 : 2600);
}

// --- Confirm dialog ----------------------------------------------------------

const confirmText = ref("");
const confirmVisible = ref(false);
let confirmResolver: ((value: boolean) => void) | null = null;

function confirmDialog(text: string): Promise<boolean> {
  confirmText.value = text;
  confirmVisible.value = true;
  return new Promise((resolve) => {
    confirmResolver = resolve;
  });
}

function resolveConfirm(value: boolean) {
  confirmVisible.value = false;
  confirmResolver?.(value);
  confirmResolver = null;
}

// --- Prompt dialog -----------------------------------------------------------

const promptText = ref("");
const promptPlaceholder = ref("");
const promptValue = ref("");
const promptVisible = ref(false);
let promptResolver: ((value: string | null) => void) | null = null;

function promptDialog(message: string, placeholder = ""): Promise<string | null> {
  promptText.value = message;
  promptPlaceholder.value = placeholder;
  promptValue.value = "";
  promptVisible.value = true;
  return new Promise((resolve) => {
    promptResolver = resolve;
  });
}

function resolvePrompt(value: string | null) {
  promptVisible.value = false;
  promptResolver?.(value);
  promptResolver = null;
}

// --- Request plumbing ----------------------------------------------------------

function authToken(): string {
  return session.value?.token ?? "";
}

function onUnauthorized(): void {
  saveSession(null);
  session.value = null;
  resetConsole();
  toast("Session expired - sign in again", "error");
}

function resetConsole(): void {
  view.value = "dashboard";
  selectedDeviceId.value = null;
  devices.value = [];
  stats.value = {};
}

/** Shared result handling: unwrap on success, force-login on 401, toast errors. */
async function handle<T>(pending: Promise<api.Result<T>>): Promise<T | null> {
  const r = await pending;
  if (r.ok) return r.value;
  if (r.status === 401) {
    onUnauthorized();
    return null;
  }
  toast(r.error || `HTTP ${r.status}`, "error");
  return null;
}

/** User-initiated action: prevents double-submit while `name` is in flight. */
async function run<T>(name: string, fn: () => Promise<api.Result<T>>): Promise<T | null> {
  if (busy.value) return null;
  busy.value = name;
  try {
    return await handle(fn());
  } finally {
    busy.value = null;
  }
}

function isBusy(name: string): boolean {
  return busy.value === name;
}

// --- Devices & dashboard -------------------------------------------------------

/** Sequence guard so a slow stale response can't clobber a newer one. */
let loadSeq = 0;

async function loadDevices() {
  const seq = ++loadSeq;
  const list = await handle(api.listDevices(authToken()));
  if (list === null || seq !== loadSeq) return;
  devices.value = list;

  const entries = await Promise.all(
    list.map(async (d) => {
      const [a, t] = await Promise.all([
        handle(api.listAlarms(authToken(), d.id)),
        handle(api.listTodos(authToken(), d.id)),
      ]);
      const alarms = a ?? [];
      const todos = t ?? [];
      return [
        d.id,
        { alarms: alarms.length, todos: todos.length, done: todos.filter((x) => x.done).length },
      ] as const;
    }),
  );
  if (seq !== loadSeq) return;
  stats.value = Object.fromEntries(entries);
}

function goDashboard() {
  view.value = "dashboard";
  loadDevices();
}

function openDevice(id: string) {
  selectedDeviceId.value = id;
  view.value = "device";
}

function onDeviceDeleted() {
  selectedDeviceId.value = null;
  view.value = "dashboard";
  loadDevices();
}

function goAccount() {
  view.value = "account";
}

// --- Session --------------------------------------------------------------------

function authenticated(next: Session): void {
  saveSession(next);
  session.value = next;
  resetConsole();
  loadDevices();
}

async function logout() {
  const s = session.value;
  if (!s) return;
  if (s.kind === "account") await api.logoutAccount(s.token);
  saveSession(null);
  session.value = null;
  resetConsole();
  toast("Signed out");
}

// --- Provide shared UI -------------------------------------------------------------

provide<UI>(uiKey, {
  session,
  authToken,
  toast,
  confirmDialog,
  promptDialog,
  run,
  isBusy,
  onUnauthorized,
});

onMounted(async () => {
  const stored = loadSession();
  if (!stored) {
    checkingSession.value = false;
    return;
  }
  const r = await api.me(stored.token);
  if (r.ok) {
    session.value = stored;
    resetConsole();
    loadDevices();
  } else {
    saveSession(null);
  }
  checkingSession.value = false;
});
</script>

<template>
  <LoginView v-if="!checkingSession && !session" @authenticated="authenticated" />

  <main v-else-if="session">
    <header class="masthead">
      <div class="masthead-title">
        <div class="eyebrow">Device cloud · admin console</div>
        <h1>Inkwash Console</h1>
        <p>Manage device content and prepare the next sync.</p>
      </div>
      <div class="userbar">
        <button class="quiet" type="button" @click="goAccount">Account</button>
        <span class="status" :class="session.kind">
          <span class="dot" aria-hidden="true"></span>
          {{ session.kind === "admin" ? "ADMIN" : "USER" }} · {{ session.username }}
        </span>
        <button class="quiet" type="button" @click="logout">Sign out</button>
      </div>
    </header>

    <DashboardView
      v-if="view === 'dashboard'"
      :devices="devices"
      :stats="stats"
      @open-device="openDevice"
      @refresh="loadDevices"
      @open-account="goAccount"
    />
    <DeviceView
      v-else-if="view === 'device' && selectedDevice"
      :device="selectedDevice"
      @back="goDashboard"
      @deleted="onDeviceDeleted"
    />
    <AccountView v-else-if="view === 'account'" @back="goDashboard" @logout="logout" />
  </main>

  <div
    v-if="confirmVisible"
    class="modal-backdrop"
    @click.self="resolveConfirm(false)"
    @keydown.esc="resolveConfirm(false)"
  >
    <div class="modal" role="alertdialog" aria-modal="true" aria-label="Confirm action">
      <p class="modal-text">{{ confirmText }}</p>
      <div class="row" style="justify-content: flex-end">
        <button type="button" class="quiet" @click="resolveConfirm(false)">Cancel</button>
        <button type="button" class="danger" autofocus @click="resolveConfirm(true)">Confirm</button>
      </div>
    </div>
  </div>

  <div
    v-if="promptVisible"
    class="modal-backdrop"
    @click.self="resolvePrompt(null)"
    @keydown.esc="resolvePrompt(null)"
  >
    <div class="modal" role="dialog" aria-modal="true" aria-label="Enter a value">
      <p class="modal-text">{{ promptText }}</p>
      <input
        v-model="promptValue"
        :placeholder="promptPlaceholder"
        class="modal-input"
        type="password"
        autocomplete="new-password"
        @keyup.enter="resolvePrompt(promptValue.trim() || null)"
      />
      <div class="row" style="justify-content: flex-end; margin-top: 16px">
        <button type="button" class="quiet" @click="resolvePrompt(null)">Cancel</button>
        <button type="button" class="primary" @click="resolvePrompt(promptValue.trim() || null)">
          Confirm
        </button>
      </div>
    </div>
  </div>

  <div class="toast" :class="[toastVariant, { show: toastVisible }]" role="status" aria-live="polite">
    {{ toastText }}
  </div>
</template>
